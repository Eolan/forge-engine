//! Connection tokens: who may connect to which server, signed by the login service and checked
//! by the server without asking it.
//!
//! The model is netcode.io's connect token (Fiedler, `netcode` 1.02): a web login hands the
//! player a short-lived token naming its client id, the servers it is good for and some user
//! data; the game server verifies it offline. netcode seals its token with a key the login and
//! the servers share; here the issuer signs it with Ed25519 (RFC 8032), so a server holds only
//! the issuer's public key and cannot mint tokens. The token travels inside QUIC's TLS 1.3
//! connection, so nobody on the path reads or replays it.
//!
//! Layout, little-endian:
//!
//! | bytes | field |
//! |---|---|
//! | 4 | `FTK1` |
//! | 8 | client id |
//! | 8 | issued, Unix seconds |
//! | 8 | expires, Unix seconds |
//! | 1 + 8·n | the servers it is good for, at most [`MAX_SERVERS`] |
//! | 1 + m | user data, at most 255 bytes |
//! | 64 | the Ed25519 signature of everything before it |

use std::time::{SystemTime, UNIX_EPOCH};

use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

use crate::wire::{Reader, Writer};

/// The token format's tag.
const TAG: [u8; 4] = *b"FTK1";
/// An Ed25519 signature's length.
const SIGNATURE: usize = 64;
/// Servers a token names at most.
pub const MAX_SERVERS: usize = 16;
/// How far a token's issue time may be ahead of the server's clock.
pub const CLOCK_SKEW: u64 = 60;

/// What a token says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenClaims {
    /// The client's id: the server's name for this player.
    pub client_id: u64,
    /// When it was issued, Unix seconds.
    pub issued: u64,
    /// When it stops being good, Unix seconds.
    pub expires: u64,
    /// The servers it is good for.
    pub servers: Vec<u64>,
    /// Whatever the login service passes on (a session, a display name).
    pub user_data: Vec<u8>,
}

impl TokenClaims {
    /// Claims for `client_id` on `server_id`, good for `lifetime` seconds from now.
    pub fn new(client_id: u64, server_id: u64, lifetime: u64) -> Self {
        let now = unix_now();
        Self {
            client_id,
            issued: now,
            expires: now + lifetime,
            servers: vec![server_id],
            user_data: Vec::new(),
        }
    }

    fn encode(&self) -> Vec<u8> {
        assert!(self.servers.len() <= MAX_SERVERS, "too many servers");
        assert!(self.user_data.len() <= 255, "too much user data");
        let mut w = Writer::new();
        w.raw(&TAG)
            .u64(self.client_id)
            .u64(self.issued)
            .u64(self.expires)
            .u8(self.servers.len() as u8);
        for s in &self.servers {
            w.u64(*s);
        }
        w.u8(self.user_data.len() as u8).raw(&self.user_data);
        w.bytes
    }

    fn decode(body: &[u8]) -> Result<Self, TokenError> {
        let mut r = Reader::new(body);
        let malformed = |_| TokenError::Malformed;
        if r.raw(4).map_err(malformed)? != TAG {
            return Err(TokenError::Malformed);
        }
        let client_id = r.u64().map_err(malformed)?;
        let issued = r.u64().map_err(malformed)?;
        let expires = r.u64().map_err(malformed)?;
        let n = r.u8().map_err(malformed)? as usize;
        if n > MAX_SERVERS {
            return Err(TokenError::Malformed);
        }
        let servers = (0..n)
            .map(|_| r.u64().map_err(malformed))
            .collect::<Result<_, _>>()?;
        let m = r.u8().map_err(malformed)? as usize;
        let user_data = r.raw(m).map_err(malformed)?.to_vec();
        r.end().map_err(malformed)?;
        Ok(Self {
            client_id,
            issued,
            expires,
            servers,
            user_data,
        })
    }
}

/// Why a token was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[repr(u8)]
pub enum TokenError {
    /// It does not parse.
    #[error("the token does not parse")]
    Malformed = 1,
    /// Its signature is not the issuer's.
    #[error("the token is not signed by the issuer")]
    BadSignature = 2,
    /// Its time is over.
    #[error("the token has expired")]
    Expired = 3,
    /// It was issued in the future (beyond [`CLOCK_SKEW`]).
    #[error("the token is not valid yet")]
    NotYetValid = 4,
    /// It does not name this server.
    #[error("the token is for another server")]
    WrongServer = 5,
}

impl TokenError {
    /// The error a byte names, if any.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Malformed),
            2 => Some(Self::BadSignature),
            3 => Some(Self::Expired),
            4 => Some(Self::NotYetValid),
            5 => Some(Self::WrongServer),
            _ => None,
        }
    }
}

/// The login service's side: holds the signing key and issues tokens.
pub struct TokenIssuer {
    key: Ed25519KeyPair,
}

impl std::fmt::Debug for TokenIssuer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenIssuer")
            .field("public_key", &self.public_key())
            .finish()
    }
}

/// A key could not be made or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the signing key could not be made or read")]
pub struct KeyError;

impl TokenIssuer {
    /// An issuer with a new key from the system's generator. Its public key goes to the
    /// servers; the key itself never leaves the login service.
    pub fn generate() -> Result<Self, KeyError> {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(|_| KeyError)?;
        Self::from_pkcs8(pkcs8.as_ref())
    }

    /// An issuer from a key stored as PKCS #8 (v2, as [`TokenIssuer::generate`] makes them).
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, KeyError> {
        Ok(Self {
            key: Ed25519KeyPair::from_pkcs8(pkcs8).map_err(|_| KeyError)?,
        })
    }

    /// The public key the servers check signatures with.
    pub fn public_key(&self) -> [u8; 32] {
        self.key
            .public_key()
            .as_ref()
            .try_into()
            .expect("Ed25519 public keys are 32 bytes")
    }

    /// A signed token carrying `claims`.
    ///
    /// # Panics
    /// With more than [`MAX_SERVERS`] servers or 255 bytes of user data.
    pub fn issue(&self, claims: &TokenClaims) -> Vec<u8> {
        let mut token = claims.encode();
        let signature = self.key.sign(&token);
        token.extend_from_slice(signature.as_ref());
        token
    }
}

/// The server's side: checks tokens with the issuer's public key.
#[derive(Clone, Debug)]
pub struct TokenVerifier {
    public_key: [u8; 32],
    server_id: u64,
}

impl TokenVerifier {
    /// A verifier for server `server_id` that trusts the issuer of `public_key`.
    pub fn new(public_key: [u8; 32], server_id: u64) -> Self {
        Self {
            public_key,
            server_id,
        }
    }

    /// The server's id.
    pub fn server_id(&self) -> u64 {
        self.server_id
    }

    /// The claims of a token that is the issuer's, current at `now` (Unix seconds) and good
    /// for this server.
    pub fn verify_at(&self, token: &[u8], now: u64) -> Result<TokenClaims, TokenError> {
        if token.len() < SIGNATURE {
            return Err(TokenError::Malformed);
        }
        let (body, signature) = token.split_at(token.len() - SIGNATURE);
        UnparsedPublicKey::new(&ED25519, &self.public_key)
            .verify(body, signature)
            .map_err(|_| TokenError::BadSignature)?;
        let claims = TokenClaims::decode(body)?;
        if now >= claims.expires {
            return Err(TokenError::Expired);
        }
        if claims.issued > now + CLOCK_SKEW {
            return Err(TokenError::NotYetValid);
        }
        if !claims.servers.contains(&self.server_id) {
            return Err(TokenError::WrongServer);
        }
        Ok(claims)
    }

    /// [`TokenVerifier::verify_at`] by the system's clock.
    pub fn verify(&self, token: &[u8]) -> Result<TokenClaims, TokenError> {
        self.verify_at(token, unix_now())
    }
}

/// The system's clock in Unix seconds.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_good_for_its_server_and_its_time_only() {
        let issuer = TokenIssuer::generate().unwrap();
        let verifier = TokenVerifier::new(issuer.public_key(), 7);
        let claims = TokenClaims {
            client_id: 42,
            issued: 1000,
            expires: 1300,
            servers: vec![3, 7],
            user_data: b"pilot".to_vec(),
        };
        let token = issuer.issue(&claims);
        assert_eq!(verifier.verify_at(&token, 1100), Ok(claims));
        assert_eq!(verifier.verify_at(&token, 1300), Err(TokenError::Expired));
        assert_eq!(
            verifier.verify_at(&token, 900),
            Err(TokenError::NotYetValid)
        );
        let other = TokenVerifier::new(issuer.public_key(), 8);
        assert_eq!(other.verify_at(&token, 1100), Err(TokenError::WrongServer));
    }

    #[test]
    fn a_forged_or_altered_token_is_refused() {
        let issuer = TokenIssuer::generate().unwrap();
        let forger = TokenIssuer::generate().unwrap();
        let verifier = TokenVerifier::new(issuer.public_key(), 1);
        let claims = TokenClaims::new(5, 1, 60);
        let forged = forger.issue(&claims);
        assert_eq!(verifier.verify(&forged), Err(TokenError::BadSignature));
        let mut altered = issuer.issue(&claims);
        altered[4] ^= 1; // another client id
        assert_eq!(verifier.verify(&altered), Err(TokenError::BadSignature));
        assert_eq!(verifier.verify(b"short"), Err(TokenError::Malformed));
        assert!(verifier.verify(&issuer.issue(&claims)).is_ok());
    }
}

//! Byte layouts: little-endian integers, QUIC's variable-length integers, and the handshake's
//! two messages.
//!
//! The handshake runs on the first bidirectional stream, which the client opens: it sends a
//! [`Hello`] and finishes the stream; the server answers with a [`Welcome`] or closes the
//! connection with a [`crate::Refusal`]'s code (see [`crate::protocol`]).

use crate::channel::Delivery;

/// Writes integers and byte strings into a growing buffer.
#[derive(Debug, Default)]
pub struct Writer {
    /// What was written.
    pub bytes: Vec<u8>,
}

impl Writer {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// One byte.
    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.bytes.push(v);
        self
    }

    /// Two bytes, little-endian.
    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Eight bytes, little-endian.
    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Raw bytes.
    pub fn raw(&mut self, v: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(v);
        self
    }

    /// QUIC's variable-length integer (RFC 9000, 16): 1, 2, 4 or 8 bytes for values under
    /// 2⁶, 2¹⁴, 2³⁰ and 2⁶².
    ///
    /// # Panics
    /// If `v` is 2⁶² or more.
    pub fn varint(&mut self, v: u64) -> &mut Self {
        varint(&mut self.bytes, v);
        self
    }
}

/// Appends `v` as a QUIC variable-length integer (see [`Writer::varint`]).
///
/// # Panics
/// If `v` is 2⁶² or more.
pub fn varint(out: &mut Vec<u8>, v: u64) {
    if v < 1 << 6 {
        out.push(v as u8);
    } else if v < 1 << 14 {
        out.extend_from_slice(&(v as u16 | 0x4000).to_be_bytes());
    } else if v < 1 << 30 {
        out.extend_from_slice(&(v as u32 | 0x8000_0000).to_be_bytes());
    } else {
        assert!(v < 1 << 62, "{v} does not fit a QUIC varint");
        out.extend_from_slice(&(v | 0xC000_0000_0000_0000).to_be_bytes());
    }
}

/// The length of a varint from its first byte.
pub fn varint_len(first: u8) -> usize {
    1 << (first >> 6)
}

/// Reads what [`Writer`] wrote; every read fails past the end.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

/// A read went past the end, or a value was out of its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("malformed message")]
pub struct Malformed;

impl<'a> Reader<'a> {
    /// A reader at the start of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// What is left.
    pub fn rest(&self) -> &'a [u8] {
        self.bytes
    }

    /// `n` raw bytes.
    pub fn raw(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.bytes.len() < n {
            return Err(Malformed);
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }

    /// One byte.
    pub fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.raw(1)?[0])
    }

    /// Two bytes, little-endian.
    pub fn u16(&mut self) -> Result<u16, Malformed> {
        Ok(u16::from_le_bytes(
            self.raw(2)?.try_into().expect("2 bytes"),
        ))
    }

    /// Eight bytes, little-endian.
    pub fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(
            self.raw(8)?.try_into().expect("8 bytes"),
        ))
    }

    /// A QUIC variable-length integer.
    pub fn varint(&mut self) -> Result<u64, Malformed> {
        let first = *self.bytes.first().ok_or(Malformed)?;
        let bytes = self.raw(varint_len(first))?;
        let mut v = u64::from(first & 0x3F);
        for b in &bytes[1..] {
            v = v << 8 | u64::from(*b);
        }
        Ok(v)
    }

    /// Fails unless everything was read.
    pub fn end(&self) -> Result<(), Malformed> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(Malformed)
        }
    }
}

/// The first bytes of every handshake message.
pub const MAGIC: [u8; 4] = *b"FRGN";

/// The client's first message: who it is and what it speaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The protocol version it speaks ([`crate::PROTOCOL_VERSION`]).
    pub version: u16,
    /// Its channels, in order: both ends must have the same.
    pub channels: Vec<Delivery>,
    /// Its connection token, as the issuer signed it.
    pub token: Vec<u8>,
}

impl Hello {
    /// Its bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(&MAGIC)
            .u16(self.version)
            .u8(self.channels.len() as u8);
        for c in &self.channels {
            w.u8(*c as u8);
        }
        w.u16(self.token.len() as u16).raw(&self.token);
        w.bytes
    }

    /// Reads one; the version is read first, so that a client of another version is told so
    /// even if the rest has changed.
    pub fn decode(bytes: &[u8]) -> Result<Self, Malformed> {
        let mut r = Reader::new(bytes);
        if r.raw(4)? != MAGIC {
            return Err(Malformed);
        }
        let version = r.u16()?;
        let n = r.u8()?;
        let channels = (0..n)
            .map(|_| Delivery::from_u8(r.u8()?).ok_or(Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        let len = r.u16()?;
        let token = r.raw(len as usize)?.to_vec();
        r.end()?;
        Ok(Self {
            version,
            channels,
            token,
        })
    }

    /// The version alone, from a hello whose rest may not parse.
    pub fn version_of(bytes: &[u8]) -> Option<u16> {
        let mut r = Reader::new(bytes);
        (r.raw(4).ok()? == MAGIC).then_some(())?;
        r.u16().ok()
    }
}

/// The server's answer to a hello it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// The protocol version both speak.
    pub version: u16,
    /// The client's id, from its token.
    pub client_id: u64,
    /// The server's id.
    pub server_id: u64,
}

impl Welcome {
    /// Its bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(&MAGIC)
            .u16(self.version)
            .u64(self.client_id)
            .u64(self.server_id);
        w.bytes
    }

    /// Reads one.
    pub fn decode(bytes: &[u8]) -> Result<Self, Malformed> {
        let mut r = Reader::new(bytes);
        if r.raw(4)? != MAGIC {
            return Err(Malformed);
        }
        let welcome = Self {
            version: r.u16()?,
            client_id: r.u64()?,
            server_id: r.u64()?,
        };
        r.end()?;
        Ok(welcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_at_every_length() {
        for v in [
            0,
            63,
            64,
            16_383,
            16_384,
            (1 << 30) - 1,
            1 << 30,
            (1 << 62) - 1,
        ] {
            let mut bytes = Vec::new();
            varint(&mut bytes, v);
            assert_eq!(bytes.len(), varint_len(bytes[0]));
            let mut r = Reader::new(&bytes);
            assert_eq!(r.varint(), Ok(v));
            assert!(r.end().is_ok());
        }
    }

    #[test]
    fn handshake_messages_round_trip_and_refuse_garbage() {
        let hello = Hello {
            version: 3,
            channels: vec![Delivery::ReliableOrdered, Delivery::Unreliable],
            token: vec![1, 2, 3],
        };
        let bytes = hello.encode();
        assert_eq!(Hello::decode(&bytes), Ok(hello));
        assert_eq!(Hello::version_of(&bytes), Some(3));
        assert_eq!(Hello::decode(&bytes[..bytes.len() - 1]), Err(Malformed));
        assert_eq!(Hello::decode(b"nonsense"), Err(Malformed));
        let welcome = Welcome {
            version: 1,
            client_id: 42,
            server_id: 7,
        };
        assert_eq!(Welcome::decode(&welcome.encode()), Ok(welcome));
    }
}

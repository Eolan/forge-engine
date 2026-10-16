//! A bit writer and reader: snapshots are packed to the bit (D-010's hand-written bit writer).
//!
//! Bits go in least significant first, into 64-bit words written out little-endian, so a
//! value of `n` bits costs exactly `n` bits wherever it falls. The reader fails, rather than
//! panics, past the end: a packet from the network may be cut or forged.

/// Packs values into bits.
#[derive(Clone, Debug, Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    scratch: u64,
    /// Bits in `scratch`.
    used: u32,
}

impl BitWriter {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bits written so far.
    pub fn bits(&self) -> usize {
        self.bytes.len() * 8 + self.used as usize
    }

    /// Bytes the writer will take once finished.
    pub fn len_bytes(&self) -> usize {
        self.bits().div_ceil(8)
    }

    /// Writes the low `n` bits of `value` (`n` up to 64).
    pub fn write(&mut self, value: u64, n: u32) {
        debug_assert!(n <= 64);
        if n == 0 {
            return;
        }
        let value = if n == 64 {
            value
        } else {
            value & ((1 << n) - 1)
        };
        self.scratch |= value << self.used;
        let room = 64 - self.used;
        if n < room {
            self.used += n;
            return;
        }
        self.bytes.extend_from_slice(&self.scratch.to_le_bytes());
        // What did not fit goes into the next word.
        self.scratch = if room == 64 { 0 } else { value >> room };
        self.used = n - room;
    }

    /// One bit.
    pub fn bool(&mut self, v: bool) {
        self.write(u64::from(v), 1);
    }

    /// A signed value in `n` bits, two's complement.
    pub fn signed(&mut self, v: i64, n: u32) {
        self.write(v as u64, n);
    }

    /// An unsigned value in groups of `group` bits, each followed by a bit saying whether
    /// another group follows: small values cost `group + 1` bits.
    pub fn varbits(&mut self, mut v: u64, group: u32) {
        loop {
            self.write(v, group);
            v >>= group;
            self.bool(v != 0);
            if v == 0 {
                return;
            }
        }
    }

    /// The `k`th 64-bit word written, the one in progress included.
    fn word(&self, k: usize) -> u64 {
        let full = self.bytes.len() / 8;
        match k.cmp(&full) {
            std::cmp::Ordering::Less => u64::from_le_bytes(
                self.bytes[8 * k..8 * k + 8]
                    .try_into()
                    .expect("eight bytes"),
            ),
            std::cmp::Ordering::Equal => self.scratch,
            std::cmp::Ordering::Greater => 0,
        }
    }

    /// `n` bits (up to 64) written from bit `at` on.
    fn peek(&self, at: usize, n: u32) -> u64 {
        let (k, s) = (at / 64, (at % 64) as u32);
        let low = self.word(k) >> s;
        let v = if s == 0 {
            low
        } else {
            low | self.word(k + 1) << (64 - s)
        };
        if n == 64 { v } else { v & ((1 << n) - 1) }
    }

    /// Appends bits `start..start + len` of what another writer wrote.
    pub fn append_range(&mut self, other: &BitWriter, start: usize, len: usize) {
        debug_assert!(start + len <= other.bits());
        let (mut at, end) = (start, start + len);
        while at < end {
            let n = (end - at).min(64) as u32;
            self.write(other.peek(at, n), n);
            at += n as usize;
        }
    }

    /// Appends everything another writer wrote.
    pub fn append(&mut self, other: &BitWriter) {
        self.append_range(other, 0, other.bits());
    }

    /// The bytes, the last one padded with zeros.
    pub fn finish(mut self) -> Vec<u8> {
        let n = self.len_bytes();
        self.bytes.extend_from_slice(&self.scratch.to_le_bytes());
        self.bytes.truncate(n);
        self.bytes
    }
}

/// A read went past the end of the packet, or a value was out of range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the packet ended early or holds a value out of range")]
pub struct BadPacket;

/// Unpacks what [`BitWriter`] packed.
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    bytes: &'a [u8],
    /// Bits read.
    at: usize,
}

impl<'a> BitReader<'a> {
    /// A reader at the first bit of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    /// Bits read so far.
    pub fn position(&self) -> usize {
        self.at
    }

    /// Bits left (the last byte's padding included).
    pub fn remaining(&self) -> usize {
        self.bytes.len() * 8 - self.at
    }

    /// Reads `n` bits (up to 64).
    pub fn read(&mut self, n: u32) -> Result<u64, BadPacket> {
        debug_assert!(n <= 64);
        if n == 0 {
            return Ok(0);
        }
        if self.remaining() < n as usize {
            return Err(BadPacket);
        }
        let (byte, shift) = (self.at / 8, (self.at % 8) as u32);
        if n + shift <= 64 && byte + 8 <= self.bytes.len() {
            // One unaligned word holds them all.
            let word = u64::from_le_bytes(self.bytes[byte..byte + 8].try_into().expect("8"));
            self.at += n as usize;
            let v = word >> shift;
            return Ok(if n == 64 { v } else { v & ((1 << n) - 1) });
        }
        let mut v = 0u64;
        let mut got = 0;
        while got < n {
            let byte = self.bytes[self.at / 8];
            let shift = (self.at % 8) as u32;
            let take = (8 - shift).min(n - got);
            let bits = (u64::from(byte) >> shift) & ((1 << take) - 1);
            v |= bits << got;
            got += take;
            self.at += take as usize;
        }
        Ok(v)
    }

    /// One bit.
    pub fn bool(&mut self) -> Result<bool, BadPacket> {
        Ok(self.read(1)? == 1)
    }

    /// A signed value of `n` bits.
    pub fn signed(&mut self, n: u32) -> Result<i64, BadPacket> {
        let v = self.read(n)?;
        Ok(if n == 64 || v >> (n - 1) == 0 {
            v as i64
        } else {
            (v | !((1u64 << n) - 1)) as i64
        })
    }

    /// What [`BitWriter::varbits`] wrote.
    pub fn varbits(&mut self, group: u32) -> Result<u64, BadPacket> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            if shift >= 64 {
                return Err(BadPacket);
            }
            v |= self.read(group)? << shift;
            shift += group;
            if !self.bool()? {
                return Ok(v);
            }
        }
    }
}

/// Folds a signed value into an unsigned one, small magnitudes small (Protocol Buffers'
/// zigzag).
pub fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// The inverse of [`zigzag`].
pub fn unzigzag(v: u64) -> i64 {
    (v >> 1) as i64 ^ -((v & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::SplitMix64;

    #[test]
    fn values_of_every_width_round_trip() {
        let mut random = SplitMix64::new(3);
        let mut w = BitWriter::new();
        let mut expected = Vec::new();
        for _ in 0..5000 {
            let n = 1 + random.below(64) as u32;
            let v = random.next_u64() & if n == 64 { u64::MAX } else { (1 << n) - 1 };
            w.write(v, n);
            expected.push((v, n));
        }
        let bits = w.bits();
        let bytes = w.finish();
        assert_eq!(bytes.len(), bits.div_ceil(8));
        let mut r = BitReader::new(&bytes);
        for (v, n) in expected {
            assert_eq!(r.read(n), Ok(v));
        }
        assert!(r.remaining() < 8);
        assert_eq!(r.read(8), Err(BadPacket));
    }

    #[test]
    fn signed_varbits_and_zigzag_round_trip() {
        let mut w = BitWriter::new();
        for v in [-5i64, 0, 7, -2048, 2047] {
            w.signed(v, 12);
        }
        for v in [0u64, 1, 15, 16, 300, u64::MAX] {
            w.varbits(v, 4);
        }
        w.bool(true);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        for v in [-5i64, 0, 7, -2048, 2047] {
            assert_eq!(r.signed(12), Ok(v));
        }
        for v in [0u64, 1, 15, 16, 300, u64::MAX] {
            assert_eq!(r.varbits(4), Ok(v));
        }
        assert_eq!(r.bool(), Ok(true));
        for v in [0i64, 1, -1, 1000, -1000, i64::MAX, i64::MIN] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
        assert_eq!((zigzag(0), zigzag(-1), zigzag(1)), (0, 1, 2));
    }

    #[test]
    fn appending_a_writer_keeps_its_bits() {
        let mut a = BitWriter::new();
        a.write(0b101, 3);
        let mut b = BitWriter::new();
        for k in 0..100u64 {
            b.write(k, 7);
        }
        a.append(&b);
        a.write(1, 1);
        assert_eq!(a.bits(), 3 + 700 + 1);
        let bytes = a.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(3), Ok(0b101));
        for k in 0..100u64 {
            assert_eq!(r.read(7), Ok(k));
        }
        assert_eq!(r.read(1), Ok(1));
    }
}

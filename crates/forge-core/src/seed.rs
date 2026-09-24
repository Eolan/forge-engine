//! Seeds and random streams for order-independent procedural generation.
//!
//! Every generated thing gets its seed from its parent's seed and a *stable* identifier
//! ([`Seed::derive`]), never from a shared random generator. Anything can therefore be
//! generated on its own, in any order, on any machine, on any thread, with the same result.
//! This is the discipline Eiserloh's "Noise-Based RNG" (GDC 2017) argues for, see
//! `docs/RESEARCH.md` §3.

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::{Xxh3, xxh3_64_with_seed};

/// Domain tags so numeric and byte-string identifiers can never produce the same child seed.
const TAG_NUMBER: u8 = 1;
const TAG_BYTES: u8 = 2;

/// A 64-bit generation seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seed(u64);

impl Seed {
    /// Wraps a raw seed value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw seed value.
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Child seed for a numeric identifier (a chunk key, an entity index…).
    ///
    /// Depends only on `self` and `stable_id`.
    #[must_use]
    pub fn derive(self, stable_id: u64) -> Self {
        let mut input = [0_u8; 9];
        input[0] = TAG_NUMBER;
        input[1..].copy_from_slice(&stable_id.to_le_bytes());
        Self(xxh3_64_with_seed(&input, self.0))
    }

    /// Child seed for a byte-string identifier.
    #[must_use]
    pub fn derive_bytes(self, stable_id: &[u8]) -> Self {
        let mut hasher = Xxh3::with_seed(self.0);
        hasher.update(&[TAG_BYTES]);
        hasher.update(stable_id);
        Self(hasher.digest())
    }

    /// Child seed for a textual identifier (`"terrain"`, `"flora"`…).
    #[must_use]
    pub fn derive_str(self, stable_id: &str) -> Self {
        self.derive_bytes(stable_id.as_bytes())
    }

    /// A random stream starting from this seed.
    pub const fn rng(self) -> SplitMix64 {
        SplitMix64::new(self.0)
    }
}

/// SplitMix64 pseudo-random generator: tiny, fast and statistically solid for generation.
///
/// Not cryptographically secure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a stream from a raw seed.
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Next 32 random bits.
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)`, from 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1_u64 << 53) as f64)
    }

    /// Uniform in `[0, 1)`, from 24 random bits.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1_u32 << 24) as f32)
    }

    /// Uniform between `low` and `high`.
    pub fn range_f64(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next_f64()
    }

    /// Uniform between `low` and `high`.
    pub fn range_f32(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next_f32()
    }

    /// A fair coin flip.
    pub fn next_bool(&mut self) -> bool {
        self.next_u64() >> 63 == 1
    }

    /// Uniform integer in `[0, bound)`, without modulo bias (Lemire's method).
    ///
    /// # Panics
    /// If `bound` is zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "`bound` must be positive");
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let product = u128::from(self.next_u64()) * u128::from(bound);
            if product as u64 >= threshold {
                return (product >> 64) as u64;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_reference_values() {
        // First outputs of the reference implementation (Vigna) seeded with 0.
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.next_u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.next_u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn derivation_depends_only_on_parent_and_identifier() {
        let root = Seed::new(42);
        let first = root.derive(7);
        let _unrelated = root.derive(8).derive_str("terrain");
        assert_eq!(root.derive(7), first);
        assert_ne!(root.derive(8), first);
        assert_ne!(Seed::new(43).derive(7), first);
        assert_eq!(root.derive_str("flora"), root.derive_bytes(b"flora"));
    }

    #[test]
    fn numeric_and_byte_identifiers_never_collide() {
        let root = Seed::new(1);
        assert_ne!(
            root.derive(0x1234),
            root.derive_bytes(&0x1234_u64.to_le_bytes())
        );
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut rng = Seed::new(3).rng();
        let mut seen = [false; 10];
        for _ in 0..1_000 {
            let value = rng.below(10);
            assert!(value < 10);
            seen[value as usize] = true;
        }
        assert!(seen.iter().all(|&hit| hit));
    }
}

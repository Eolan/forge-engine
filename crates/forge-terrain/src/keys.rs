//! Where the island's products are kept between starts and their keys (#208, D-053): a
//! product's key digests its inputs (its world's settings, its upstream products' keys) and the
//! code that makes it (`build.rs`), so a change to either remakes it and what follows from it.

use forge_core::derived::{DerivedCache, Key, KeyHasher};
use forge_procgen::Field2;

use crate::IslandWorld;

/// The island's products' code digests (#208), written by `build.rs`.
pub(crate) mod code_digests {
    include!(concat!(env!("OUT_DIR"), "/code_digests.rs"));
}

/// Where the island's products are kept between starts (#208, D-053): beside the cooked meshes
/// until #213 gathers every cache under `cache/`.
pub(crate) fn derived_cache() -> DerivedCache {
    DerivedCache::new(crate::workspace_root().join("mesh-cache/derived"))
}

/// The key of the island's eroded field (#208) in world `w`: its parameters.
pub(crate) fn eroded_key(w: &IslandWorld) -> Key {
    KeyHasher::new()
        .debug(&forge_procgen::island::island_key(&w.island, &w.erosion))
        .key(code_digests::CODE_ERODED)
}

/// The key of [`island_heights`]: the eroded field's, and how it is shaped.
pub(crate) fn heights_key(w: &IslandWorld) -> Key {
    KeyHasher::new()
        .number(eroded_key(w).digest())
        .debug(&w.valleys)
        .debug(&w.rivers)
        .debug(&(
            w.ground.smoothing,
            w.ground.sea_floor,
            w.ground.shore_smoothing,
        ))
        .key(code_digests::CODE_HEIGHTS)
}

/// The key of [`island_water`] made from the heights of key `heights`.
pub(crate) fn water_key(heights: Key, w: &IslandWorld) -> Key {
    KeyHasher::new()
        .number(heights.digest())
        .debug(&w.rivers)
        .debug(&w.channels)
        .key(code_digests::CODE_WATER)
}

/// The key of the stones of `seed` on the water of key `water`.
pub(crate) fn stones_key(water: Key, seed: u64) -> Key {
    KeyHasher::new()
        .number(water.digest())
        .number(seed)
        .key(code_digests::CODE_STONES)
}

/// The key of [`island_layers`].
pub(crate) fn layers_key(w: &IslandWorld, water: bool) -> Key {
    KeyHasher::new()
        .number(water_key(heights_key(w), w).digest())
        // The island's seed sets its rocks' hardness.
        .debug(&w.island)
        // Every layer rule and switch, the map's size and the sand's height among them.
        .debug(&w.layers)
        .debug(&water)
        .key(code_digests::CODE_LAYERS)
}

/// The key of the amplified field made from the heights of key `heights`. The field is not
/// stored (268 MB, 3.5 s to make, read only to make the drawn ground): its key is part of the
/// drawn ground's, so a change to the amplification remakes that.
pub(crate) fn amplified_derived_key(heights: Key, factor: u32, seed: u64) -> Key {
    KeyHasher::new()
        .number(heights.digest())
        .number(u64::from(factor))
        .number(seed)
        .key(code_digests::CODE_AMPLIFIED)
}

/// How many times finer than the field world `w`'s ground is drawn (#106; `IslandWorld::parse`
/// checks the factor divides the channels' split).
pub fn drawn_factor(w: &IslandWorld) -> u32 {
    (w.island.spacing / w.ground.drawn_spacing).round().max(1.0) as u32
}

/// The key of [`island_drawn`]: the water's, the amplified field's when it adds detail, and how
/// they are drawn.
pub(crate) fn drawn_key(w: &IslandWorld) -> Key {
    let heights = heights_key(w);
    let factor = drawn_factor(w);
    let mut key = KeyHasher::new().number(water_key(heights, w).digest());
    if factor > 1 && w.ground.detail > 0.0 {
        key = key.number(amplified_derived_key(heights, factor, w.island.seed.value()).digest());
    }
    key.debug(&(factor, w.ground)).key(code_digests::CODE_DRAWN)
}

/// The identity of a field in a process: what the products made from a field alone are
/// memoized by.
pub(crate) fn field_identity(height: &Field2<f32>) -> u64 {
    height.digest() ^ height.spacing.to_bits() ^ u64::from(height.size)
}

/// The keys of the heights [`island_heights`] made, by their field's identity: the products
/// made from a field alone (the water, the stones, the amplified field) find their upstream key
/// here. A field made otherwise has none, and its products are made, not stored.
pub(crate) static HEIGHTS_KEYS: std::sync::Mutex<Vec<(u64, Key)>> =
    std::sync::Mutex::new(Vec::new());

/// The key of the heights `height` was made as, if [`island_heights`] made it.
pub(crate) fn heights_key_of(identity: u64) -> Option<Key> {
    HEIGHTS_KEYS
        .lock()
        .expect("the heights' keys")
        .iter()
        .find(|(made, _)| *made == identity)
        .map(|(_, key)| *key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The island's products' keys, in their order downstream.
    fn keys(w: &IslandWorld) -> [Key; 5] {
        [
            eroded_key(w),
            heights_key(w),
            water_key(heights_key(w), w),
            layers_key(w, true),
            drawn_key(w),
        ]
    }

    /// Which of [`keys`] differ between the code's world and the world `change` makes of it.
    fn moved(change: impl FnOnce(&mut IslandWorld)) -> [bool; 5] {
        let a = IslandWorld::default();
        let mut b = a.clone();
        change(&mut b);
        let (a, b) = (keys(&a), keys(&b));
        std::array::from_fn(|i| a[i] != b[i])
    }

    #[test]
    fn a_parameter_change_remakes_only_the_products_that_read_it() {
        // The salt: the layers' alone.
        assert_eq!(
            moved(|w| w.layers.salt = None),
            [false, false, false, true, false]
        );
        // The amplification's strength: the drawn ground's alone.
        assert_eq!(
            moved(|w| w.ground.detail = 0.5),
            [false, false, false, false, true]
        );
        // The valleys reshape the heights: everything after the eroded field.
        assert_eq!(moved(|w| w.valleys = None), [false, true, true, true, true]);
        // The coastal plain shapes the erosion: everything.
        assert_eq!(moved(|w| w.island.plain = 0.3), [true; 5]);
        // The same world, the same keys.
        assert_eq!(moved(|_| {}), [false; 5]);
    }
}

//! The stones the island's water lays (#105, #118): boulders in its rivers' beds and on the
//! gravel beside the steeper ones.

use forge_procgen::Field2;

use crate::keys::{derived_cache, field_identity, heights_key_of, stones_key, water_key};
use crate::world::world;

/// `seed`'s stones on the island's water, kept between starts (#208) for the heights
/// [`island_heights`] made.
fn derived_stones(
    product: &str,
    height: &Field2<f32>,
    seed: u64,
    make: impl FnOnce() -> Vec<forge_procgen::Stone>,
) -> Vec<forge_procgen::Stone> {
    match heights_key_of(field_identity(height)) {
        Some(heights) => {
            derived_cache()
                .get_or_make(product, stones_key(water_key(heights, world()), seed), make)
                .value
        }
        None => make(),
    }
}

/// The seed of the stones in the island's rivers.
pub const RIVER_STONES: u64 = 0x5705_e105;

/// The seed of the rubble on the island's scree (#118).
pub const SCREE_RUBBLE: u64 = 0x5c2e_e2ab;

/// The stones in the island's rivers (#105): boulders on the carved beds, which the water flows
/// around. Made once a process for the island's heights (the loading thread makes them, #201).
pub fn island_stones(
    height: &Field2<f32>,
    ribbons: &[forge_procgen::Ribbon],
    channels: &forge_procgen::Channels,
) -> Vec<forge_procgen::Stone> {
    static MADE: std::sync::Mutex<Option<(u64, Vec<forge_procgen::Stone>)>> =
        std::sync::Mutex::new(None);
    let key = height.digest() ^ u64::from(height.size);
    let mut made = MADE.lock().expect("the island's river stones");
    if let Some((made_for, stones)) = made.as_ref()
        && *made_for == key
    {
        return stones.clone();
    }
    let stones = derived_stones("island-stones", height, RIVER_STONES, || {
        forge_procgen::stones(ribbons, channels, height, RIVER_STONES)
    });
    *made = Some((key, stones.clone()));
    stones
}

/// The stones beside the steeper rivers' water, on their gravel (#118), made once a process for
/// the island's heights (#201).
pub fn island_bank_stones(
    height: &Field2<f32>,
    ribbons: &[forge_procgen::Ribbon],
    channels: &forge_procgen::Channels,
) -> Vec<forge_procgen::Stone> {
    static MADE: std::sync::Mutex<Option<(u64, Vec<forge_procgen::Stone>)>> =
        std::sync::Mutex::new(None);
    let key = height.digest() ^ u64::from(height.size);
    let mut made = MADE.lock().expect("the island's bank stones");
    if let Some((made_for, stones)) = made.as_ref()
        && *made_for == key
    {
        return stones.clone();
    }
    let stones = derived_stones("island-bank-stones", height, RIVER_STONES ^ 0xba, || {
        forge_procgen::bank_stones(ribbons, channels, height, RIVER_STONES ^ 0xba)
    });
    *made = Some((key, stones.clone()));
    stones
}

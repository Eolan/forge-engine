//! The island's water on the land (#105, D-038): its rivers as ribbons, the channels carved for
//! them and its lakes as level planes.

use forge_procgen::Field2;
use forge_task::TaskPool;

use crate::heights::{island_lake_waters, island_rivers};
use crate::keys::{derived_cache, field_identity, heights_key_of, water_key};
use crate::world::world;

/// The island's water on the land (#105, D-038's rivers and lakes).
#[derive(Clone)]
pub struct IslandWater {
    /// Each river smoothed into a ribbon of points 4 m apart with its width, depth, speed and
    /// level, the tributaries first.
    pub ribbons: Vec<forge_procgen::Ribbon>,
    /// The channels carved for them, which the island's mesh draws (and the ribbons rest on far
    /// away).
    pub channels: forge_procgen::Channels,
    /// The lakes of a hectare or more, a level plane each over the samples it covers.
    pub lakes: Vec<forge_procgen::LakeWater>,
}

forge_core::stored!(IslandWater {
    ribbons,
    channels,
    lakes
});

/// The island's rivers and lakes as water and beds, once a process for the field it was made
/// from: the ground's layers and the water both ask for it at start (0.75 s each). Kept
/// between starts (#208) for the heights [`island_heights`](crate::island_heights) made.
pub fn island_water(height: &Field2<f32>) -> IslandWater {
    static MADE: std::sync::Mutex<Option<(u64, IslandWater)>> = std::sync::Mutex::new(None);
    let key = field_identity(height);
    let mut made = MADE.lock().expect("the island's water");
    if let Some((made_for, water)) = made.as_ref()
        && *made_for == key
    {
        return water.clone();
    }
    let water = match heights_key_of(key) {
        Some(heights) => {
            let derived =
                derived_cache().get_or_make("island-water", water_key(heights, world()), || {
                    make_island_water(height)
                });
            tracing::info!(
                from_cache = derived.from_cache,
                ms = derived.ms,
                "the island's water"
            );
            derived.value
        }
        None => make_island_water(height),
    };
    *made = Some((key, water.clone()));
    water
}

/// [`island_water`], made.
fn make_island_water(height: &Field2<f32>) -> IslandWater {
    let flow = forge_procgen::drain(height, 0.0, &TaskPool::client());
    let rivers = island_rivers(height, &flow);
    let (_, mut lakes) = island_lake_waters(height, &flow);
    let mut ribbons = forge_procgen::ribbons(height, &rivers, &lakes, &world().rivers);
    let sills = world().channels.sill.is_some();
    let channels = forge_procgen::Channels::new(height, &ribbons, &lakes, &world().channels);
    // The lakes' water off the shallow arms past their outlets, which rise into sills (#120).
    if sills {
        let trimmed = forge_procgen::trim_outlets(&mut lakes, height, &ribbons);
        tracing::info!(trimmed, "the lakes' outlets' arms trimmed (samples)");
    }
    forge_procgen::rest_on(
        &mut ribbons,
        &|x, y| channels.height_at(height, x, y),
        &TaskPool::client(),
    );
    IslandWater {
        ribbons,
        channels,
        lakes,
    }
}

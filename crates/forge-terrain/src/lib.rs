//! Forge terrain on the CPU (D-053, issue #212): a world's description in
//! ([`world::IslandWorld`], read from its TOML file), its products out. Each product is made
//! once a process and kept between starts by its inputs and its code (#208,
//! `forge_core::derived`); deterministic on every machine (D-016), over `forge-procgen`'s
//! generation and `forge-geom`'s cooking. The upload to the GPU, the materials and the water's
//! GPU records stay with the renderer and the demos.
//!
//! - [`mod@world`]: the description, its file and the process's world ([`world::world`]).
//! - [`heights`]: the eroded field shaped for the sea and carved for the rivers' valleys.
//! - [`water`]: the rivers as ribbons, their channels and the lakes.
//! - [`stones`]: the boulders in the rivers and on their gravel.
//! - [`layers`]: the layer map and where the loose rocks lie.
//! - [`drawn`]: the ground as its tiles draw it, finer, with the amplification's detail.
//! - [`tiles`]: the ground's tiles as props to cook.
//! - [`warm_island`]: all of them on the loading thread.
//! - [`planet`]: a planet's ground as tiles of the cube sphere (#220, D-056).

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use forge_task::TaskPool;

pub mod drawn;
pub mod heights;
mod keys;
pub mod layers;
pub mod planet;
pub mod stones;
pub mod tiles;
pub mod water;
pub mod world;

pub use drawn::{DrawnGround, island_drawn};
pub use heights::island_heights;
pub use keys::drawn_factor;
pub use layers::{IslandLayers, island_layer, island_layers};
pub use planet::{Planet, PlanetWorld, planet_file};
pub use stones::{RIVER_STONES, SCREE_RUBBLE, island_bank_stones, island_stones};
pub use tiles::{ISLAND_TILES, island_tiles};
pub use water::{IslandWater, island_water};
pub use world::{IslandWorld, world};

use drawn::amplify_ahead;
use keys::{derived_cache, drawn_key};

/// The workspace's root: where `assets/` and the caches are.
fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `relative` (a world file's path to a map) under the workspace's root.
pub fn workspace_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

/// The island's CPU work, done on the loading thread while the loading screen draws (#201): its
/// heights, its water, then its drawn ground, its rivers' stones and its layers side by side. Each is made once a
/// process, so the finishing step on the main thread, which needs the device, finds them made;
/// before, it made them there behind a frozen screen (about 13 s of 15 with a warm cache).
/// `draws_water` as [`island_layers`] takes it; `beside` runs beside the drawn ground and the
/// layers, after the stones (the demo cooks its sand window there).
pub fn warm_island(draws_water: bool, beside: impl FnOnce() + Send) {
    let start = Instant::now();
    let height = island_heights();
    let factor = drawn_factor(world());
    std::thread::scope(|ahead| {
        // The drawn ground's amplified detail needs the heights alone: beside the water. Not
        // when the drawn ground is stored (#208): only its making reads the field.
        let drawn_stored = derived_cache()
            .path("island-drawn", drawn_key(world()))
            .exists();
        if factor > 1 && world().ground.detail > 0.0 && !drawn_stored {
            ahead.spawn(|| {
                amplify_ahead(
                    &height,
                    factor,
                    world().island.seed.value(),
                    &TaskPool::client(),
                );
            });
        }
        let water = island_water(&height);
        std::thread::scope(|threads| {
            threads.spawn(island_drawn);
            threads.spawn(|| {
                island_stones(&height, &water.ribbons, &water.channels);
                island_bank_stones(&height, &water.ribbons, &water.channels);
                beside();
            });
            island_layers(draws_water);
        });
    });
    tracing::info!(
        ms = start.elapsed().as_millis(),
        "the island's heights, water, drawn ground and layers, behind the loading screen (#201)"
    );
}

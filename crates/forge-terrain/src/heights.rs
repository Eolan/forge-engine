//! The island's heights: the eroded field, shaped for the sea (its floor, the shore smoothed)
//! and carved for the rivers' valleys; and the rivers and lakes traced over a field.

use std::time::Instant;

use forge_procgen::Field2;
use forge_task::TaskPool;

use crate::keys::{HEIGHTS_KEYS, derived_cache, eroded_key, field_identity, heights_key};
use crate::world::world;

/// The island's heightfield: the eroded field, shaped for the sea and the rivers. Both are
/// kept between starts (#208); made or loaded once a process (the sea, the camera, the layers
/// and the cook all ask for it).
pub fn island_heights() -> Field2<f32> {
    static MADE: std::sync::Mutex<Option<(String, Field2<f32>)>> = std::sync::Mutex::new(None);
    let (params, erosion) = (world().island, world().erosion);
    let key = format!(
        "{} {:?} {:?}",
        forge_procgen::island::island_key(&params, &erosion),
        world().valleys,
        world().rivers
    );
    let mut made = MADE.lock().expect("the island's heights");
    if let Some((made_for, height)) = made.as_ref()
        && *made_for == key
    {
        return height.clone();
    }
    let start = Instant::now();
    let derived = derived_cache().get_or_make("island-heights", heights_key(world()), || {
        make_island_heights()
    });
    let height = derived.value;
    let (lo, hi) = height.min_max();
    tracing::info!(
        seed = world().island.seed.value(),
        samples = height.size,
        spacing_m = height.spacing,
        from_cache = derived.from_cache,
        ms = start.elapsed().as_millis(),
        height_m = %format_args!("{lo:.0}-{hi:.0}"),
        "island heightfield"
    );
    HEIGHTS_KEYS
        .lock()
        .expect("the heights' keys")
        .push((field_identity(&height), heights_key(world())));
    *made = Some((key, height.clone()));
    height
}

/// [`island_heights`], made: the eroded field (stored on its own, the longest to make), then
/// shaped.
fn make_island_heights() -> Field2<f32> {
    let (params, erosion) = (world().island, world().erosion);
    // The eroded fields kept before #208 (`island-<key>.f32` among the meshes, keyed without their
    // code) are read no more.
    if let Ok(dir) = std::fs::read_dir(forge_core::derived::cache_dir(
        forge_core::derived::CacheKind::Meshes,
    )) {
        for path in dir.filter_map(|e| e.ok().map(|e| e.path())) {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name.starts_with("island-") && name.ends_with(".f32") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let pool = TaskPool::client();
    let eroded = derived_cache().get_or_make("island-eroded", eroded_key(world()), || {
        forge_procgen::generate_island(&params, &erosion, &pool).0
    });
    tracing::info!(
        from_cache = eroded.from_cache,
        ms = eroded.ms,
        "the island's eroded field"
    );
    // The sea floor under the flat sea (#96): the sea's plane then meets the ground along the
    // coast, between the samples.
    let mut height = eroded.value;
    forge_procgen::smooth_shore(&mut height, 0.0, f32::INFINITY, world().ground.smoothing);
    let coast = forge_procgen::coast_distance(&height, 0.0, &pool);
    forge_procgen::sea_floor(
        &mut height,
        &coast,
        0.0,
        world().ground.sea_floor.0,
        world().ground.sea_floor.1,
    );
    // The ground within a few metres of the sea's level smoothed, so the coast runs smooth
    // instead of stepping with the samples (#106).
    forge_procgen::smooth_shore(
        &mut height,
        0.0,
        world().ground.shore_smoothing.0,
        world().ground.shore_smoothing.1,
    );
    // The rivers' valleys (#116, D-041): a floor for each river's water and, on its gentler
    // reaches, a bench or a floodplain, from the rivers traced over the field so far. The
    // rivers are traced again over the carved field (`island_water`).
    if let Some(valleys) = world().valleys {
        let carve = Instant::now();
        let flow = forge_procgen::drain(&height, 0.0, &pool);
        let rivers = island_rivers(&height, &flow);
        let (lakes, waters) = island_lake_waters(&height, &flow);
        // Without their steps (#122): the floors follow the rivers' fall, not their pools, so
        // the 8 m field is the same with them or without.
        let stepless = forge_procgen::RibbonParams {
            steps: None,
            ..world().rivers
        };
        let ribbons = forge_procgen::ribbons(&height, &rivers, &waters, &stepless);
        let s = forge_procgen::carve_valleys(&mut height, &ribbons, &lakes, &valleys, &pool);
        tracing::info!(
            points = %format_args!("{} floodplain, {} bench, {} room, {} in lakes", s.floodplain, s.bench, s.room, s.in_lake),
            floor_m = %format_args!("{:.1} of {:.1} asked", s.mean_floor.0, s.mean_floor.1),
            lowered = s.lowered,
            deepest_cut_m = %format_args!("{:.1}", s.deepest_cut),
            guarded = s.guarded,
            ms = carve.elapsed().as_millis(),
            "the rivers' valleys carved"
        );
    }
    height
}

/// The island's rivers (stage 4 on the drawn field, as `genesis` traces them): where more than
/// 0.5 km² drains through a sample of `flow`.
pub(crate) fn island_rivers(
    height: &Field2<f32>,
    flow: &forge_procgen::Flow,
) -> forge_procgen::Rivers {
    let min_area = (500_000.0 / (height.spacing * height.spacing)) as u32 + 1;
    forge_procgen::trace_rivers(height, flow, min_area)
}

/// The island's lakes of a hectare or more, as `genesis` traces them: the priority flood's
/// water standing over half a metre above the drawn field.
pub(crate) fn island_lakes(
    height: &Field2<f32>,
    flow: &forge_procgen::Flow,
) -> forge_procgen::Lakes {
    let filled = forge_procgen::priority_flood(height, 0.0);
    forge_procgen::trace_lakes(height, &filled, flow, 0.5)
}

/// [`island_lakes`] and their water: the lakes of the rivers' `lake_area` or more as level
/// planes over the samples they stand over, which the rivers run into.
pub(crate) fn island_lake_waters(
    height: &Field2<f32>,
    flow: &forge_procgen::Flow,
) -> (forge_procgen::Lakes, Vec<forge_procgen::LakeWater>) {
    let filled = forge_procgen::priority_flood(height, 0.0);
    let lakes = forge_procgen::trace_lakes(height, &filled, flow, 0.5);
    let waters = forge_procgen::lake_waters(height, &filled, &lakes, world().rivers.lake_area);
    (lakes, waters)
}

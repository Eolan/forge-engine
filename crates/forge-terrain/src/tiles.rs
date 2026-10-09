//! The island's ground tiles (#106): the drawn ground as props for `forge_geom` to cook.

use std::sync::Arc;

use forge_geom::city::{CellWindow, Heightfield, PropKind, PropSpec};

use crate::drawn::island_drawn;
use crate::keys::{code_digests, drawn_factor, drawn_key};
use crate::world::world;

/// Tiles a side of the island's ground (#106): 2 km each over the 16 km.
pub const ISLAND_TILES: u32 = 8;

/// The island's ground as props (on its layered ground, `CityMaterials::island_ground`), its
/// samples generated only when a tile's cooked mesh is not in the cache. In tiles (#106), each
/// cooked and cached on its own, their borders locked in every level so they meet without a
/// crack; named `island@x-z`, drawn finer `island@x-z-2m` (the cache keeps one file per name,
/// so the two stay side by side; the material is the name's before the `@`).
pub fn island_tiles() -> Vec<PropSpec> {
    let (params, erosion) = (world().island, world().erosion);
    let factor = drawn_factor(world());
    let size = (params.size - 1) * factor + 1;
    let spacing = params.spacing / f64::from(factor);
    let mut key = format!(
        "{}, smoothed {} passes, sea floor {} m over {} m, shore smoothed {:?}, rivers {:?} carved {:?} in valleys {:?}",
        forge_procgen::island::island_key(&params, &erosion),
        world().ground.smoothing,
        world().ground.sea_floor.0,
        world().ground.sea_floor.1,
        world().ground.shore_smoothing,
        world().rivers,
        world().channels,
        world().valleys,
    );
    if factor > 1 {
        key += &format!(", drawn on the cubic at {spacing} m");
        if world().ground.detail > 0.0 {
            key += &format!(
                ", amplified {:?} x {} faded over {:?} m",
                forge_procgen::AmplifyParams::island(forge_core::Seed::new(0)),
                world().ground.detail,
                world().ground.detail_fade
            );
        }
    }
    // The drawn ground's key and the cooking code's digest (#208): the tiles follow a change
    // of either, as the island's other products do.
    key += &format!(
        ", drawn {:016x}, cooked by {:016x}",
        drawn_key(world()).digest(),
        code_digests::CODE_TILES
    );
    let source: Arc<dyn Fn() -> Arc<[f32]> + Send + Sync> =
        Arc::new(|| island_drawn().heights.clone());
    // The rivers' channels, carved into cells drawn in quads of a metre (#105).
    let detail: Arc<forge_geom::city::DetailSource> =
        Arc::new(|_: &[f32]| island_drawn().detail.clone());
    let cells = size - 1;
    let edges: Vec<u32> = (0..=ISLAND_TILES)
        .map(|t| t * cells / ISLAND_TILES)
        .collect();
    let mut tiles = Vec::new();
    for tz in 0..ISLAND_TILES as usize {
        for tx in 0..ISLAND_TILES as usize {
            tiles.push(PropSpec {
                name: if factor > 1 {
                    format!("island@{tx}-{tz}-{spacing}m")
                } else {
                    format!("island@{tx}-{tz}")
                },
                kind: PropKind::Heightfield(Heightfield {
                    key: key.clone(),
                    samples: size,
                    spacing: spacing as f32,
                    source: source.clone(),
                    detail: Some(detail.clone()),
                    window: Some(CellWindow {
                        first: [edges[tx], edges[tz]],
                        cells: [edges[tx + 1] - edges[tx], edges[tz + 1] - edges[tz]],
                    }),
                }),
            });
        }
    }
    tiles
}

//! Forge procedural generation on the CPU (Phase 2, `docs/ROADMAP.md`; the pipeline of
//! `docs/research/terrain-genesis.md`, "Recommendation for Forge"). Everything is a pure
//! function of a seed and a parameter record, deterministic on every machine (D-016): noise
//! from integer-lattice hashes, integer drainage areas, pinned tie-breaks, `forge_core::dmath`
//! for the transcendental functions.
//!
//! - [`field`]: `Field2<T>`, a square grid of samples with a spacing in metres.
//! - [`noise`]: gradient noise on an integer lattice, `fbm` and ridges.
//! - [`island`]: stage 1 and 2 of the terrain pipeline, the island's mask and its uplift,
//!   hardness and rain fields.
//! - [`flow`]: D8 receivers, the downstream-first stack and integer drainage areas (Braun &
//!   Willett 2013); depressions by the basin graph (Cordonnier, Bovy & Braun 2019) every
//!   step, by priority flood (Barnes 2014) for the reference and the lakes.
//! - [`erosion`]: the implicit stream-power law with hillslope diffusion, uplift against
//!   erosion until mountains and valleys appear, rows and drainage trees in parallel on the
//!   job system.
//! - [`amplify`]: stage 5, the eroded field at half its spacing with a detail erosion under the
//!   coarse drainage, in tiles with halos that give the untiled field to the bit.
//! - [`hydrology`]: stage 4, the river network as polylines with Strahler orders and widths
//!   from the catchment, and the lakes with their levels and outlets.
//! - [`coast`]: the signed distance to the coast, what the shore's water keys on.
//! - [`ocean`]: the open sea's directional spectrum (JONSWAP/TMA) and its inverse FFT on the
//!   CPU, the reference the GPU's cascades are diffed against.
//! - [`shore`]: the shore's waves: the floor's depth against the coast distance, and per train
//!   of waves the time its crests take to the shore and their shoaling, for the GPU.
//! - [`river`]: the rivers' water: each river's course smoothed into a ribbon of points with
//!   its width, depth, speed and a level that only falls, for the GPU.
//! - [`channel`]: the rivers' channels carved under that water, on cells drawn finer.
//! - [`valley`]: the rivers' valleys carved into the field: a floor for the water, a bench or a
//!   floodplain on the gentler reaches, the walls' foot lowered to meet it; and the steep
//!   valleys' ground painted: gravel beds, scree at the walls' foot, scrub on the walls.
//! - [`lake`]: the lakes' water, a level plane each and the mask of the samples it covers.
//! - [`layers`]: stage 6's first rule, the ground's material layers from slope and altitude.
//! - [`preview`]: PNG previews of any stage (height, hillshade, flow, an overview with the
//!   sea, rivers and lakes), which is how the pipeline is looked at before a GPU draws it.
//!
//! `cargo run --release -p genesis` (tools/genesis) runs the pipeline and writes the previews;
//! `city-blocks --island SEED` cooks the island's heightfield into a cluster DAG and draws it.

#![forbid(unsafe_code)]

pub mod amplify;
pub mod channel;
pub mod coast;
pub mod erosion;
pub mod field;
pub mod flow;
pub mod hydrology;
pub mod island;
pub mod lake;
pub mod layers;
pub mod noise;
pub mod ocean;
pub mod preview;
pub mod river;
pub mod shore;
pub mod valley;

pub use amplify::{AmplifyParams, amplify};
pub use channel::{
    ChannelParams, Channels, FineGround, Stone, bank_stones, paint_banks, paint_beds, paint_fans,
    stones,
};
pub use coast::{coast_distance, sea_floor, site_distance, smooth_shore};
pub use erosion::{Erosion, ErosionParams, erode};
pub use field::Field2;
pub use flow::{Drainage, Flow, drain, priority_flood, route};
pub use hydrology::{
    DepressionFill, Lake, Lakes, Mouth, River, Rivers, fill_small_depressions, trace_lakes,
    trace_rivers,
};
pub use island::{
    IslandFields, IslandParams, Wind, cached_island, generate_island, island_fields,
    orographic_rain, refresh_rain,
};
pub use lake::{LakeWater, lake_waters, paint_lake_beds};
pub use layers::{
    LayerRule, ScrubRule, Shore, paint_lakes, paint_moisture, paint_rivers, paint_scrub,
    slope_layers, wetness,
};
pub use ocean::{Ocean, OceanParams, OceanSurface, tma};
pub use river::{
    Corner, Delta, DeltaParams, Ribbon, RibbonParams, RibbonPoint, Step, StepParams, drawn_height,
    lip_shift, rest_on, ribbons, sea_mouth, smooth_height,
};
pub use shore::{BREAKER_INDEX, ShoreProfile, ShoreTrain, wave_number};
pub use valley::{
    ValleyGround, ValleyPainted, ValleyParams, ValleyStats, carve_valleys, paint_valley_ground,
};

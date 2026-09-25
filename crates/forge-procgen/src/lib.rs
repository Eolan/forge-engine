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
//! - [`layers`]: stage 6's first rule, the ground's material layers from slope and altitude.
//! - [`preview`]: PNG previews of any stage (height, hillshade, flow, an overview with the
//!   sea, rivers and lakes), which is how the pipeline is looked at before a GPU draws it.
//!
//! `cargo run --release -p genesis` (tools/genesis) runs the pipeline and writes the previews;
//! `city-blocks --island SEED` cooks the island's heightfield into a cluster DAG and draws it.

#![forbid(unsafe_code)]

pub mod erosion;
pub mod field;
pub mod flow;
pub mod island;
pub mod layers;
pub mod noise;
pub mod preview;

pub use erosion::{Erosion, ErosionParams, erode};
pub use field::Field2;
pub use flow::{Drainage, Flow, drain, priority_flood, route};
pub use island::{IslandFields, IslandParams, cached_island, generate_island, island_fields};
pub use layers::{LayerRule, slope_layers};

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
//! - [`flow`]: priority flood (Barnes 2014), D8 receivers, the downstream-first stack and
//!   integer drainage areas (Braun & Willett 2013).
//! - [`erosion`]: the implicit stream-power law with hillslope diffusion, uplift against
//!   erosion until mountains and valleys appear.
//! - [`preview`]: PNG previews of any stage (height, hillshade, flow, an overview with the
//!   sea, rivers and lakes), which is how the pipeline is looked at before a GPU draws it.
//!
//! `cargo run --release -p genesis` (tools/genesis) runs the pipeline and writes the previews.

#![forbid(unsafe_code)]

pub mod erosion;
pub mod field;
pub mod flow;
pub mod island;
pub mod noise;
pub mod preview;

pub use erosion::{ErosionParams, erode};
pub use field::Field2;
pub use flow::{Flow, priority_flood, route};
pub use island::{IslandFields, IslandParams, island_fields};

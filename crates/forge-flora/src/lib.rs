//! Forge's flora off the GPU (#269, D-062's parallel sessions): plants as data and rules,
//! tested headless and drawn on the CPU. Their rendering (D-013's ladder, wind) stays with the
//! renderer.
//!
//! - [`species`]: a species as data, the published sizes and a few growth parameters; the
//!   Mediterranean village's seven in `data/mediterranean.toml`.
//! - [`grow`]: a plant grown by space colonisation (Runions et al. 2007) with the
//!   self-organising vigour of Pałubicki et al. 2009, from its age, seed, site, pruning and
//!   wind, into a branch graph.
//! - [`graph`]: the branch graph (nodes, parents, radii by the pipe model, leaf clusters) and
//!   its measures and checks.
//! - [`place`]: plants placed on a heightfield: the ground's fields (slope, heat by aspect, soil,
//!   moisture), each species' habitat, the light under the canopy, spacing by pairs of species;
//!   planted rows and groves (the city's settings) and the wild layers in one pass.
//! - [`draw`]: CPU drawings: the species side by side from the side and from above.
//! - [`map`]: a placed hillside drawn from above with a section from north to south.
//!
//! Everything is a pure function of its inputs and seed, the same on every machine (D-016).

#![forbid(unsafe_code)]

pub mod draw;
pub mod graph;
pub mod grow;
pub mod map;
pub mod place;
pub mod species;

pub use graph::{LeafCluster, Node, TreeGraph};
pub use grow::{Conditions, Pruning, grow};
pub use place::{Fields, Ground, Placement, Plant, Rule, place};
pub use species::{Species, mediterranean};

#[cfg(test)]
mod tests;

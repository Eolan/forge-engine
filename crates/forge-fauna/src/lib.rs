//! Forge's fauna off the GPU (#269, D-062's parallel sessions): the village's and the city's
//! animals as behaviour, stepped at a fixed rate and tested headless. Their bodies and their
//! animation stay with the renderer.
//!
//! - [`species`]: a species as data: its size, speeds, group and flight distances; the village's
//!   eight in `data/fauna.toml` (house sparrows, feral pigeons, yellow-legged gulls, common
//!   swifts, cats, dogs, goats, thicklip grey mullet).
//! - [`plan`]: what the animals see of a place: buildings, walls, gardens, squares, streets, the
//!   water and its quay, a pasture and its fold, `forge-flora`'s trees and shrubs; the places
//!   they rest and feed in; an example village by the sea.
//! - [`passers`]: people and vehicles going by, what the animals react to; an example day.
//! - [`sim`]: groups that decide by the hour and members that move: flocks in Couzin's zones
//!   over a topological neighbourhood, looking ahead for walls; herds inside their fence; cats
//!   in their yards; dogs on the leash; schools along the quay; flight from threats by species.
//! - [`map`]: a day's paths drawn on the plan.
//!
//! Everything is a pure function of the plan, the passers and the seed, the same on every
//! machine (D-016).

#![forbid(unsafe_code)]

pub mod map;
pub mod passers;
pub mod plan;
pub mod sim;
pub mod species;

pub use passers::{Disturber, Passers, Threat};
pub use plan::{Parts, Plan};
pub use sim::{Activity, Fauna, Mode, Roster};
pub use species::{Behaviour, Species, village};

#[cfg(test)]
mod tests;

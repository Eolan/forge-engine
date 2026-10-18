//! Missions as data (D-072, D-073, #287): what the player is asked, where, with what and in
//! which order, written as a record (RON, D-035) and evaluated by Rust at the simulation's fixed
//! tick.
//!
//! - [`schema`]: the record. Objectives with the text the player reads, places (boxes and balls),
//!   items the steps spawn and the player picks up, integer counters, and one tree of steps:
//!   waits on conditions (inside a place, near one or an item, holding an item, a time, a
//!   counter, an event the game sends, all of, any of, not) that succeed or fail, combined in
//!   sequence, all together, in a race or in the background, as Verse's structured concurrency
//!   combines them, with actions (spawn, set and add, emit, end) and optional steps.
//! - [`Runtime`]: the record compiled and its state, ticked with the player's position. It
//!   tells what changed ([`MissionEvent`]), where the waits point ([`Runtime::targets`]), and
//!   saves exactly (a snapshot for rollback and replays, with its digest) or by stage (which
//!   steps ended and how, by id, for save games and hot reload into a changed record).
//!
//! The evaluation is plain integer and `f64` comparisons in a fixed order, so a mission inside a
//! deterministic simulation stays deterministic (D-016): the same ticks and positions, the same
//! state to the bit. Scripts (Luau, D-072) never run here; they send the game's events.

#![forbid(unsafe_code)]

mod runtime;
pub mod schema;

pub use runtime::{Ended, ItemState, MissionEvent, Outcome, Runtime, Stage, Status, StepStatus};
pub use schema::{
    Action, Compare, Condition, End, Item, Mission, Objective, Place, SCHEMA_VERSION, Shape, Step,
    parse, step_ids, validate,
};

/// What can go wrong with a mission.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The record does not read as a mission.
    #[error("mission: {0}")]
    Parse(String),
    /// The record reads but does not hold together (an id missing, a size negative).
    #[error("mission: {0}")]
    Invalid(String),
    /// A saved state or stage does not fit the record.
    #[error("mission state: {0}")]
    State(String),
}

#[cfg(test)]
mod tests;

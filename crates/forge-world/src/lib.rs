//! Forge world frames: where things are, at every scale (Phase 2, `docs/ROADMAP.md`; D-004).
//!
//! - [`frame`]: `f64` positions in a tree of reference frames — sectors of an integer grid, star
//!   systems, bodies, constructs — walked to express a position in any other frame, exactly
//!   across sectors.
//! - [`cells`]: the last step towards the GPU, an integer cell and an `f32` offset (issue #93),
//!   which the renderer's instance table and frame block store.
//! - [`partition`]: the flat grid and the cube sphere (equi-angular faces) that cut a world into
//!   square cells at quadtree levels, and [`cell_id`], the `u64` names of those cells.
//! - [`streaming`]: which cells a viewer needs, by level and distance, and the loads and unloads
//!   that follow the viewer with hysteresis.
//!
//! Everything here is deterministic (D-016): integer arithmetic where it can be, `f64` where
//! it cannot, and `forge_core::dmath` for the transcendental functions, so a client and a
//! server name the same cell for the same position.

#![forbid(unsafe_code)]

pub mod cell_id;
pub mod cells;
pub mod frame;
pub mod partition;
pub mod streaming;

pub use cell_id::{CellId, CellKind, MAX_LEVEL};
pub use cells::{CELL_SIZE, CellPos};
pub use frame::{Frame, FrameId, FrameKind, FrameTree, SECTOR_SIZE, SectorId, WorldPos};
pub use partition::{CubeSphere, Face, FlatGrid, Partition};
pub use streaming::{CellWant, Residency, StreamPlan, StreamPolicy};

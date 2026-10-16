//! The signal processing the mixer runs: the master's limiter, the filters behind distance
//! and occlusion, and the reverb on aux buses.
//!
//! Every processor here owns its state in buffers sized when it is made; processing a block
//! never allocates.

mod filter;
mod limiter;
mod reverb;

pub use filter::{BAND_EDGES, OnePole, ThreeBand};
pub use limiter::{Limiter, LimiterParams};
pub use reverb::{MAX_PREDELAY, MAX_SIZE, Reverb, ReverbParams};

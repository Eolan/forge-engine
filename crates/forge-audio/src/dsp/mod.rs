//! The signal processing the mixer runs: the master's limiter, the filters behind distance
//! and occlusion, the reverb on aux buses, the medium a bus is heard through; and the
//! pieces the sounds made in code are built from ([`synth`]).
//!
//! Every processor here owns its state in buffers sized when it is made; processing a block
//! never allocates.

mod filter;
mod limiter;
mod medium;
mod reverb;
pub mod synth;

pub use filter::{BAND_EDGES, Biquad, OnePole, SvfLowPass, ThreeBand};
pub use limiter::{Limiter, LimiterParams};
pub use medium::{BusFilter, MediumFilter, OPEN_CUTOFF};
pub use reverb::{MAX_PREDELAY, MAX_SIZE, Reverb, ReverbParams};

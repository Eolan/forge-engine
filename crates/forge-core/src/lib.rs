//! Forge engine foundations.
//!
//! Everything here must be bit-identical on every platform and thread count: the client and
//! the server both generate the world from the same seeds and must agree without exchanging
//! data. Hence no platform math functions ([`dmath`] instead), no stateful shared random
//! generators ([`seed`] instead) and hashes that are specified to the bit ([`hash`]).

#![forbid(unsafe_code)]

pub mod dmath;
pub mod hash;
pub mod id;
pub mod material;
pub mod seed;

pub use id::Handle;
pub use material::{Material, MaterialId, MaterialTable, RenderLayer, ShadingClass, TextureId};
pub use seed::{Seed, SplitMix64};

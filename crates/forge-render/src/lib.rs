//! Forge renderer.
//!
//! Phase 0: the meshlet path (task/mesh shaders, frustum + cone + two-pass hierarchical-Z
//! occlusion, statistics) and a starfield background. Everything is driven from
//! device-address buffers and the global bindless set of `forge-gpu`.

#![forbid(unsafe_code)]

pub mod blit;
pub mod meshlet;
pub mod starfield;
pub mod taa;
pub mod visibility;

pub use blit::blit;
pub use meshlet::{
    CullCamera, CullFlags, DrawTargets, FrameStats, MeshId, MeshletRenderer, MeshletScene,
    MeshletSceneBuilder,
};
pub use starfield::Starfield;
pub use taa::{HDR_FORMAT, Taa, TaaFrame};

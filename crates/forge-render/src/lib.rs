//! Forge renderer.
//!
//! The meshlet path (task/mesh shaders, frustum + cone + two-pass hierarchical-Z
//! occlusion, the visibility buffer and its compute resolve), a starfield background, TAA,
//! physical exposure with histogram metering and the display transform. Everything is driven from
//! device-address buffers and the global bindless set of `forge-gpu`.

#![forbid(unsafe_code)]

pub mod blit;
pub mod display;
pub mod exposure;
pub mod meshlet;
pub mod starfield;
pub mod taa;
pub mod visibility;

pub use blit::blit;
pub use display::{Display, Tonemap};
pub use exposure::{AutoExposure, LuminanceHistogram, LuminanceMeter, exposure_from_ev100};
pub use meshlet::{
    CullCamera, CullFlags, DrawTargets, FrameStats, MeshId, MeshletRenderer, MeshletScene,
    MeshletSceneBuilder,
};
pub use starfield::Starfield;
pub use taa::{HDR_FORMAT, Taa, TaaFrame};

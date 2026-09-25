//! Forge renderer.
//!
//! The meshlet path (task/mesh shaders, frustum + cone + two-pass hierarchical-Z
//! occlusion, the visibility buffer and its compute resolve), a starfield background with a
//! planet under a physical atmosphere, TAA or DLSS, physical exposure with histogram metering and the
//! display transform. Everything is driven from device-address buffers and the global
//! bindless set of `forge-gpu`.

#![forbid(unsafe_code)]

pub mod atmosphere;
pub mod blit;
pub mod bloom;
pub mod display;
pub mod exposure;
pub mod material;
pub mod meshlet;
pub mod mipcheck;
pub mod placement;
pub mod sky;
pub mod starfield;
pub mod streaming;
pub mod taa;
pub mod textures;
pub mod upscale;
pub mod visibility;

pub use atmosphere::{Atmosphere, AtmosphereFrame, AtmosphereParams};
pub use blit::blit;
pub use bloom::Bloom;
pub use display::{Display, Tonemap};
pub use exposure::{AutoExposure, LuminanceHistogram, LuminanceMeter, exposure_from_ev100};
pub use forge_gpu::DlssMode;
pub use meshlet::{
    CullCamera, CullFlags, DrawTargets, FrameStats, GeometryPath, MeshId, MeshletRenderer,
    MeshletScene, MeshletSceneBuilder, SwRaster,
};
pub use sky::{GroundSky, SkyParams};
pub use starfield::Starfield;
pub use streaming::{Residency, StreamingConfig, StreamingStats};
pub use taa::{HDR_FORMAT, Taa, TaaFrame};
pub use upscale::{DlssUpscaler, UpscaleCamera};

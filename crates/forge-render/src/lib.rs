//! Forge renderer.
//!
//! The meshlet path (task/mesh shaders, frustum + cone + two-pass hierarchical-Z
//! occlusion, the visibility buffer and its compute resolve), a starfield background with a
//! planet under a physical atmosphere, diffuse light from probes updated by ray queries, TAA
//! or DLSS, physical exposure with histogram metering and the display transform. Everything
//! is driven from device-address buffers and the global bindless set of `forge-gpu`.

#![forbid(unsafe_code)]

pub mod aces2;
pub mod atmosphere;
pub mod blit;
pub mod bloom;
pub mod cells;
pub mod clouds;
pub mod debug_hash;
pub mod display;
pub mod dust;
pub mod exposure;
pub mod ffx_shadows;
pub mod gtao;
pub mod liquid;
pub mod material;
pub mod meshlet;
pub mod mipcheck;
pub mod night;
pub mod placement;
pub mod precision;
pub mod probes;
pub mod raytrace;
pub mod reflection_history;
pub mod shallow;
pub mod skin;
pub mod sky;
pub mod skybody;
pub mod skybox;
pub mod splashes;
pub mod starfield;
pub mod streaming;
pub mod sun_shadow;
pub mod taa;
pub mod textures;
pub mod tonecheck;
pub mod upscale;
pub mod visibility;
pub mod wakes;
pub mod water;

pub use atmosphere::{Atmosphere, AtmosphereFrame, AtmosphereParams};
pub use blit::blit;
pub use bloom::Bloom;
pub use cells::{CELL_SIZE, CellPos};
pub use clouds::{CloudParams, CloudShadow, Clouds};
pub use display::{Display, HdrOutput, OutputEncoding, ToneTables, Tonemap};
pub use dust::{DustParams, DustVolume};
pub use exposure::{AutoExposure, LuminanceHistogram, LuminanceMeter, exposure_from_ev100};
pub use forge_gpu::DlssMode;
pub use gtao::{Gtao, GtaoParams};
pub use liquid::{
    FRESH_FOAM_LIFE, LIQUID_MAX_OBSTACLES, LIQUID_MAX_SUBSTEPS, Liquid, LiquidDrawParams,
    LiquidHole, LiquidLook, LiquidMode, LiquidObstacle, LiquidSolver, LiquidState, LiquidStats,
    LiquidStep, LiquidTank,
};
pub use meshlet::{
    AmbientLight, CullCamera, CullFlags, DrawTargets, FrameStats, GeometryPath, InstanceOcclusion,
    MeshId, MeshletRenderer, MeshletScene, MeshletSceneBuilder, MoverTransform, MoversFrame,
    PlanetView, RayRequests, SunShadowRays, SwRaster,
};
pub use probes::{ProbeLight, ProbeParams, Probes};
pub use reflection_history::{ReflectionHistory, ReflectionHistoryFrame};
pub use shallow::{
    ShallowColumns, ShallowGrid, ShallowLayer, ShallowParams, ShallowState, ShallowStats,
};
pub use skin::{HeightField, SkinBlend, skin_vertex};
pub use sky::{GroundSky, SkyFrame, SkyLight, SkyNight, SkyParams, sh_irradiance};
pub use skybody::{SkyBody, SkyBodyView};
pub use skybox::{SkyBox, SkyBoxPlanet, sky_from_body};
pub use splashes::{
    SPLASH_CAPACITY, SplashFoam, SplashParams, SplashSource, SplashStats, WaterSplashes,
};
pub use starfield::Starfield;
pub use streaming::{Residency, StartView, StreamingConfig, StreamingStats};
pub use sun_shadow::{
    DENOISED_SUN_RADIUS, ShadowDenoiserKind, SunShadowCamera, SunShadowDenoiser, SunShadowFrame,
};
pub use taa::{HDR_FORMAT, Taa, TaaFrame};
pub use upscale::{DlssUpscaler, UpscaleCamera};
pub use wakes::{MAX_WAKES, WakeFrame, WaterWake, WaterWakes};
pub use water::{
    MAX_FLOATERS, MAX_POOL_SAMPLES, SeaWater, WATER_MIPS, WATER_SIZE, WaterCascadeDesc,
    WaterCascades, WaterCaustics, WaterFloater, WaterFrame, WaterLake, WaterMouth, WaterPool,
    WaterPoolOnGpu, WaterRiverPoint, WaterSample, WaterShore, WaterShoreTrain, WaterStone,
    WaterSurface, WaterSurfaceParams, WetGround,
};

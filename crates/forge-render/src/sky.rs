//! The sky seen from inside an atmosphere (issue #43, `shaders/sky.slang`), after Hillaire
//! 2020: the ground-view half of D-023, whose tables ([`crate::Atmosphere`]) it reads.
//!
//! Three graph passes a frame:
//! - `sky/sky-view table`: the light the air scatters towards the camera, 192 × 108
//!   directions around it (elevation squashed towards the horizon, azimuth from the sun's);
//! - `sky/aerial perspective`: the light gathered and the transmittance from the camera to 32
//!   depths along 32 × 32 view rays (a volume laid out as a 1024 × 32 atlas);
//! - `sky/compose`: where the depth is empty, the sky from the table and the sun's disc
//!   through the air; elsewhere the colour dimmed by the transmittance and lit by the air up
//!   to the pixel's distance.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    Result, ShaderCompiler, ShaderStage, vk,
};
use glam::{Mat4, Vec3};

use crate::atmosphere::AtmosphereFrame;

const SKY_VIEW_SIZE: [u32; 2] = [192, 108];
/// Froxels per side of an aerial-perspective slice, and slices (`SLICE`, `SLICES`).
const SLICE: u32 = 32;
const SLICES: u32 = 32;
const GROUP: u32 = 8;

/// Mirrors `Sky` in `sky.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuSky {
    inv_view_proj: [f32; 16],
    camera: [f32; 4],
    sun: [f32; 4],
    scale: [f32; 4],
    transmittance: u32,
    multiple_scattering: u32,
    sky_view: u32,
    sky_view_storage: u32,
    aerial: u32,
    aerial_storage: u32,
    depth: u32,
    color: u32,
    width: u32,
    height: u32,
    pad: [u32; 2],
    planet: u64,
}

const _: () = assert!(std::mem::size_of::<GpuSky>() == 168);

/// What a frame's sky needs besides the atmosphere.
#[derive(Clone, Copy, Debug)]
pub struct SkyParams {
    /// The drawing camera's view-projection (jitter included), world metres.
    pub view_proj: Mat4,
    /// The camera, world metres.
    pub camera: Vec3,
    /// Towards the sun, world axes.
    pub sun_dir: Vec3,
    /// The sun disc's angular radius, radians.
    pub sun_angular_radius: f32,
    /// Pre-exposed luminance of a unit of sun illuminance: the sun's illuminance (lux, above
    /// the atmosphere) times the frame's exposure.
    pub luminance_scale: f32,
    /// How far the aerial-perspective volume reaches, km (farther pixels take its last slice).
    pub aerial_far_km: f32,
}

/// The ground view's tables, their passes and the per-frame parameters.
pub struct GroundSky {
    sky_view_pipeline: Pipeline,
    aerial_pipeline: Pipeline,
    compose_pipeline: Pipeline,
    sky_view: GraphImage,
    aerial: GraphImage,
    params: Vec<Buffer>,
}

impl GroundSky {
    /// Compiles the passes and creates the two tables.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("sky.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: 8,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let image = |size: [u32; 2], name: &str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: size[0],
                    height: size[1],
                    format: vk::Format::R16G16B16A16_SFLOAT,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let params = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuSky>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("sky {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            sky_view_pipeline: compute("sky_view_main", "sky-view table")?,
            aerial_pipeline: compute("aerial_main", "aerial perspective")?,
            compose_pipeline: compute("compose_main", "sky compose")?,
            sky_view: image(SKY_VIEW_SIZE, "sky-view table")?,
            aerial: image([SLICE * SLICES, SLICE], "aerial perspective")?,
            params,
        })
    }

    /// Declares the three passes: the tables for this frame's camera and sun, then the
    /// compose into `color` (the resolved HDR image, read and written) behind and over what
    /// `depth` shows.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        atmosphere: &AtmosphereFrame,
        params: SkyParams,
        depth: ImageHandle,
        color: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let sky_view = graph.import(&self.sky_view);
        let aerial = graph.import(&self.aerial);
        let buffer: &'f Buffer = &self.params[slot.index];
        let address = buffer.address();
        let (transmittance, multiple_scattering) =
            (atmosphere.transmittance, atmosphere.multiple_scattering);
        let planet = atmosphere.planet;
        let sun = params.sun_dir.normalize_or(Vec3::Y);
        let inverse = params.view_proj.inverse();
        let pipeline = &self.sky_view_pipeline;
        graph
            .pass("sky/sky-view table")
            .image(transmittance, ImageAccess::Sampled(compute))
            .image(multiple_scattering, ImageAccess::Sampled(compute))
            .image(sky_view, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                // Every index is known once the graph is compiled: write the frame's block.
                buffer.write(
                    0,
                    &[GpuSky {
                        inv_view_proj: inverse.to_cols_array(),
                        camera: params.camera.extend(1.0).to_array(),
                        sun: sun.extend(params.sun_angular_radius.cos()).to_array(),
                        scale: [
                            params.luminance_scale,
                            1.0 / (std::f32::consts::PI
                                * params.sun_angular_radius
                                * params.sun_angular_radius),
                            params.aerial_far_km,
                            0.0,
                        ],
                        transmittance: resources.sampled(transmittance).0,
                        multiple_scattering: resources.sampled(multiple_scattering).0,
                        sky_view: resources.sampled(sky_view).0,
                        sky_view_storage: resources.storage(sky_view, 0).0,
                        aerial: resources.sampled(aerial).0,
                        aerial_storage: resources.storage(aerial, 0).0,
                        depth: resources.sampled(depth).0,
                        color: resources.storage(color, 0).0,
                        width: extent.width,
                        height: extent.height,
                        pad: [0; 2],
                        planet,
                    }],
                );
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                let [w, h] = SKY_VIEW_SIZE;
                commands.dispatch(w.div_ceil(GROUP), h.div_ceil(GROUP), 1);
                Ok(())
            });
        let pipeline = &self.aerial_pipeline;
        graph
            .pass("sky/aerial perspective")
            .image(transmittance, ImageAccess::Sampled(compute))
            .image(multiple_scattering, ImageAccess::Sampled(compute))
            .image(aerial, ImageAccess::StorageWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(SLICE.div_ceil(GROUP), SLICE.div_ceil(GROUP), 1);
                Ok(())
            });
        let pipeline = &self.compose_pipeline;
        graph
            .pass("sky/compose")
            .image(transmittance, ImageAccess::Sampled(compute))
            .image(sky_view, ImageAccess::Sampled(compute))
            .image(aerial, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(color, ImageAccess::StorageReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(
                    extent.width.div_ceil(GROUP),
                    extent.height.div_ceil(GROUP),
                    1,
                );
                Ok(())
            });
    }
}

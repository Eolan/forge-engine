//! Sunlit dust between the rocks (issue #58, `shaders/dust.slang`): a froxel volume in front of
//! the camera whose dust scatters the sun's light towards it, with shafts where a shadow ray per
//! froxel meets a rock in the scene's TLAS.
//!
//! Three graph passes a frame:
//! - `dust/light`: per froxel (160 × 90 × 64, quadratic in depth to the volume's far end), the
//!   dust's extinction and the sunlight it scatters (Henyey–Greenstein), through a shadow ray;
//!   the sample point jitters within the froxel over TAA's cycle;
//! - `dust/integrate`: per froxel column, the light gathered and the transmittance to each
//!   slice;
//! - `dust/apply`: per pixel, the colour dimmed by the dust in front of it and brightened by
//!   what the dust scatters, by the pixel's depth.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler,
    ShaderStage, TransientDesc, vk,
};
use glam::{Mat4, Vec3};

const FROXELS_X: u32 = 160;
const FROXELS_Y: u32 = 90;
const SLICES: u32 = 64;
const GROUP: u32 = 8;

/// Mirrors `Dust` in `dust.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuDust {
    inv_view_proj: [f32; 16],
    camera: [f32; 4],
    sun: [f32; 4],
    sun_color: [f32; 4],
    params: [f32; 4],
    fill: [f32; 4],
    tlas: u64,
    light: u32,
    light_sampled: u32,
    volume: u32,
    volume_sampled: u32,
    depth: u32,
    color: u32,
    width: u32,
    height: u32,
    frame: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuDust>() == 192);

/// What a frame's dust needs.
#[derive(Clone, Copy, Debug)]
pub struct DustParams {
    /// The drawing camera's view-projection (jitter included), world metres.
    pub view_proj: Mat4,
    /// The camera, world metres.
    pub camera: Vec3,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sunlight's colour.
    pub sun_color: Vec3,
    /// The sun's illuminance times the frame's exposure: the pre-exposed light a unit of
    /// scattering returns.
    pub sun_luminance: f32,
    /// Extinction of the densest dust, per metre (all of it scatters).
    pub extinction: f32,
    /// How far the volume reaches, metres (farther pixels and the sky take all of it).
    pub far: f32,
    /// Henyey–Greenstein anisotropy: 0 scatters evenly, towards 1 mostly forwards.
    pub anisotropy: f32,
    /// Light scattered from the surroundings, pre-exposed units per unit of extinction.
    pub fill: Vec3,
    /// The scene's top-level acceleration structure (0: no shafts).
    pub tlas: u64,
    /// The noise's frame (TAA's, modulo its jitter period).
    pub frame: u32,
}

/// The passes' pipelines and the per-frame parameters.
pub struct DustVolume {
    light: Pipeline,
    integrate: Pipeline,
    apply: Pipeline,
    params: Vec<Buffer>,
}

impl DustVolume {
    /// Compiles the passes; `light_main` traces, so the device needs ray queries.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("dust.slang", entry, ShaderStage::Compute)?,
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
        let params = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuDust>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("dust {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            light: compute("light_main", "dust light")?,
            integrate: compute("integrate_main", "dust integrate")?,
            apply: compute("apply_main", "dust apply")?,
            params,
        })
    }

    /// Declares the three passes over `color` (the HDR image, read and written) behind and
    /// over what `depth` shows.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        params: DustParams,
        depth: ImageHandle,
        color: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let atlas = |name: &'static str| TransientDesc {
            name,
            width: FROXELS_X * SLICES,
            height: FROXELS_Y,
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let light = graph.transient(atlas("dust light"));
        let volume = graph.transient(atlas("dust volume"));
        let buffer: &'f Buffer = &self.params[slot.index];
        let address = buffer.address();
        let inverse = params.view_proj.inverse();
        let pipeline = &self.light;
        graph
            .pass("dust/light")
            .image(light, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                buffer.write(
                    0,
                    &[GpuDust {
                        inv_view_proj: inverse.to_cols_array(),
                        camera: params.camera.extend(1.0).to_array(),
                        sun: params
                            .sun_dir
                            .normalize_or(Vec3::Y)
                            .extend(params.sun_luminance)
                            .to_array(),
                        sun_color: params.sun_color.extend(0.0).to_array(),
                        params: [params.extinction, params.far, params.anisotropy, 1.0 / 45.0],
                        fill: params.fill.extend(0.0).to_array(),
                        tlas: params.tlas,
                        light: resources.storage(light, 0).0,
                        light_sampled: resources.sampled(light).0,
                        volume: resources.storage(volume, 0).0,
                        volume_sampled: resources.sampled(volume).0,
                        depth: resources.sampled(depth).0,
                        color: resources.storage(color, 0).0,
                        width: extent.width,
                        height: extent.height,
                        frame: params.frame,
                        pad: 0,
                    }],
                );
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(FROXELS_X.div_ceil(GROUP), FROXELS_Y.div_ceil(GROUP), SLICES);
                Ok(())
            });
        let pipeline = &self.integrate;
        graph
            .pass("dust/integrate")
            .image(light, ImageAccess::StorageReadWrite(compute))
            .image(volume, ImageAccess::StorageWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(FROXELS_X.div_ceil(GROUP), FROXELS_Y.div_ceil(GROUP), 1);
                Ok(())
            });
        let pipeline = &self.apply;
        graph
            .pass("dust/apply")
            .image(volume, ImageAccess::Sampled(compute))
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

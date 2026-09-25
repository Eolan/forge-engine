//! The sky seen from inside an atmosphere (issue #43, `shaders/sky.slang`), after Hillaire
//! 2020: the ground-view half of D-023, whose tables ([`crate::Atmosphere`]) it reads.
//!
//! Four graph passes a frame, the first three ([`GroundSky::tables`]) before the resolve:
//! - `sky/sky-view table`: the light the air scatters towards the camera, 192 × 108
//!   directions around it (elevation squashed towards the horizon, azimuth from the sun's);
//! - `sky/irradiance`: that table projected on nine spherical harmonics, the light a surface
//!   receives from the sky and the ground for its normal ([`SkyLight`], issue #47);
//! - `sky/aerial perspective`: the light gathered and the transmittance from the camera to 32
//!   depths along 32 × 32 view rays (a volume laid out as a 1024 × 32 atlas);
//!
//! and the last ([`GroundSky::compose`]) after it:
//! - `sky/compose`: where the depth is empty, the sky from the table and the sun's disc
//!   through the air; elsewhere the colour dimmed by the transmittance and lit by the air up
//!   to the pixel's distance.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle,
    MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};
use glam::{Mat4, Vec3};

use crate::atmosphere::AtmosphereFrame;

const SKY_VIEW_SIZE: [u32; 2] = [192, 108];
/// Froxels per side of an aerial-perspective slice, and slices (`SLICE`, `SLICES`).
const SLICE: u32 = 32;
const SLICES: u32 = 32;
const GROUP: u32 = 8;
/// The sky's irradiance: nine spherical-harmonic coefficients of 16 bytes (`SH_COEFFICIENTS`
/// in `sh.slang`).
const IRRADIANCE_BYTES: u64 = 9 * 16;

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
    irradiance: u64,
}

const _: () = assert!(std::mem::size_of::<GpuSky>() == 176);

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

/// The sky's irradiance for the resolve (issue #47): nine spherical-harmonic coefficients,
/// already convolved with the clamped cosine, of the light the sky and the ground send, per
/// unit of sun illuminance (`sh.slang`). Written by `sky/irradiance` every frame.
#[derive(Clone, Copy, Debug)]
pub struct SkyLight {
    /// The coefficients' buffer in this frame's graph (the resolve declares its read).
    pub buffer: BufferHandle,
    /// Its device address.
    pub address: u64,
}

/// A frame's sky between [`GroundSky::tables`] and [`GroundSky::compose`].
#[derive(Clone, Copy, Debug)]
pub struct SkyFrame {
    transmittance: ImageHandle,
    sky_view: ImageHandle,
    aerial: ImageHandle,
    address: u64,
    /// The sky's irradiance, for the resolve.
    pub light: SkyLight,
}

/// The ground view's tables, their passes and the per-frame parameters.
pub struct GroundSky {
    sky_view_pipeline: Pipeline,
    irradiance_pipeline: Pipeline,
    aerial_pipeline: Pipeline,
    compose_pipeline: Pipeline,
    sky_view: GraphImage,
    aerial: GraphImage,
    irradiance: GraphBuffer,
    params: Vec<Buffer>,
}

impl GroundSky {
    /// Compiles the passes and creates the two tables and the irradiance buffer.
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
        let irradiance = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: IRRADIANCE_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Frame,
            name: "sky irradiance",
        })?);
        Ok(Self {
            sky_view_pipeline: compute("sky_view_main", "sky-view table")?,
            irradiance_pipeline: compute("irradiance_main", "sky irradiance")?,
            aerial_pipeline: compute("aerial_main", "aerial perspective")?,
            compose_pipeline: compute("compose_main", "sky compose")?,
            sky_view: image(SKY_VIEW_SIZE, "sky-view table")?,
            aerial: image([SLICE * SLICES, SLICE], "aerial perspective")?,
            irradiance,
            params,
        })
    }

    /// Declares the tables' passes for this frame's camera and sun: the sky-view table, the
    /// sky's irradiance and the aerial perspective. `depth` and `color` are the images
    /// [`Self::compose`] will read and write.
    #[allow(clippy::too_many_arguments)]
    pub fn tables<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        atmosphere: &AtmosphereFrame,
        params: SkyParams,
        depth: ImageHandle,
        color: ImageHandle,
        extent: vk::Extent2D,
    ) -> SkyFrame {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let sky_view = graph.import(&self.sky_view);
        let aerial = graph.import(&self.aerial);
        let irradiance = graph.import_buffer(&self.irradiance);
        let irradiance_address = self.irradiance.address();
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
                        irradiance: irradiance_address,
                    }],
                );
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                let [w, h] = SKY_VIEW_SIZE;
                commands.dispatch(w.div_ceil(GROUP), h.div_ceil(GROUP), 1);
                Ok(())
            });
        let pipeline = &self.irradiance_pipeline;
        graph
            .pass("sky/irradiance")
            .image(sky_view, ImageAccess::Sampled(compute))
            .buffer(irradiance, BufferAccess::ShaderWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(1, 1, 1);
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
        SkyFrame {
            transmittance,
            sky_view,
            aerial,
            address,
            light: SkyLight {
                buffer: irradiance,
                address: irradiance_address,
            },
        }
    }

    /// Declares the compose into `color` (the resolved HDR image, read and written) behind
    /// and over what `depth` shows: the images given to [`Self::tables`].
    pub fn compose<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        sky: &SkyFrame,
        depth: ImageHandle,
        color: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let pipeline = &self.compose_pipeline;
        let address = sky.address;
        graph
            .pass("sky/compose")
            .image(sky.transmittance, ImageAccess::Sampled(compute))
            .image(sky.sky_view, ImageAccess::Sampled(compute))
            .image(sky.aerial, ImageAccess::Sampled(compute))
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

impl GroundSky {
    /// The sky's irradiance coefficients as the last submitted frame wrote them (rgb per
    /// coefficient). Waits for the device: for logs and tests, not every frame.
    pub fn read_irradiance(&self, device: &Arc<Device>) -> Result<[Vec3; 9]> {
        let bytes = device.read_back(&self.irradiance, 0, IRRADIANCE_BYTES)?;
        let values: &[[f32; 4]] = bytemuck::cast_slice(&bytes[..IRRADIANCE_BYTES as usize]);
        Ok(std::array::from_fn(|k| Vec3::from_slice(&values[k][..3])))
    }
}

/// Irradiance at the unit normal `n` from coefficients convolved with the clamped cosine:
/// `sh_irradiance` in `sh.slang`.
pub fn sh_irradiance(coefficients: &[Vec3; 9], n: Vec3) -> Vec3 {
    let b = sh_basis(n);
    coefficients
        .iter()
        .zip(b)
        .fold(Vec3::ZERO, |e, (c, b)| e + *c * b)
        .max(Vec3::ZERO)
}

/// The real SH basis of bands 0–2 at the unit direction `d`: `sh_basis` in `sh.slang`.
fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// Projects `radiance` the way `irradiance_main` does (a Fibonacci sphere, each band
    /// scaled by the clamped cosine's factor).
    fn project(radiance: impl Fn(Vec3) -> f32) -> [Vec3; 9] {
        let count = 4096;
        let mut sum = [0.0_f32; 9];
        for i in 0..count {
            let z = 1.0 - (2.0 * i as f32 + 1.0) / count as f32;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phi = i as f32 * 2.399_963_2;
            let d = Vec3::new(r * phi.cos(), z, r * phi.sin());
            for (s, b) in sum.iter_mut().zip(sh_basis(d)) {
                *s += radiance(d) * b;
            }
        }
        let band = [PI, 2.0 * PI / 3.0, PI / 4.0];
        std::array::from_fn(|k| {
            Vec3::splat(sum[k] * 4.0 * PI / count as f32 * band[[0, 1, 1, 1, 2, 2, 2, 2, 2][k]])
        })
    }

    #[test]
    fn a_uniform_sky_gives_pi_times_its_radiance_to_every_normal() {
        let c = project(|_| 1.0);
        for n in [Vec3::Y, -Vec3::Y, Vec3::X, Vec3::new(0.6, 0.0, -0.8)] {
            assert!((sh_irradiance(&c, n).x - PI).abs() < 1e-3, "{n}");
        }
    }

    #[test]
    fn a_bright_upper_hemisphere_lights_the_roof_and_half_lights_the_walls() {
        // Radiance 1 above the horizon, 0 below: the exact irradiance is π on a roof, π/2 on
        // a wall and 0 on a floor; three bands come within ~0.1 π (Ramamoorthi and Hanrahan
        // bound the error for any lighting at about 9 % of the average).
        let c = project(|d| if d.y > 0.0 { 1.0 } else { 0.0 });
        let e = |n: Vec3| sh_irradiance(&c, n).x;
        assert!((e(Vec3::Y) - PI).abs() < 0.1 * PI, "{}", e(Vec3::Y));
        assert!((e(Vec3::X) - PI / 2.0).abs() < 0.02, "{}", e(Vec3::X));
        assert!(e(-Vec3::Y) < 0.1 * PI, "{}", e(-Vec3::Y));
    }
}

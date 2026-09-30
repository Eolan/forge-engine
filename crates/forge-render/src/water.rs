//! The sea's waves on the GPU (issue #105, D-038, `shaders/water.slang`): cascades of FFT
//! waves, each a tiling patch of [`WATER_SIZE`]² samples evolved to the frame's time and
//! transformed on the async compute queue into two persistent images: the displacement (x,
//! height, y) and the slopes with the Jacobian, which marks the whitecaps (Tessendorf 2001).
//! Four graph passes a frame, each over every cascade:
//! - `water/evolve`: the spectrum at the frame's time, the fields' spectra packed two to a
//!   complex value;
//! - `water/fft-rows` and `water/fft-cols`: the inverse transform, radix-2 Stockham in
//!   groupshared memory, a workgroup per line;
//! - `water/derive`: the fields unpacked into the images.
//!
//! The spectrum comes from the CPU (`forge_procgen::Ocean::gpu_samples`, uploaded once), so
//! the GPU transforms the CPU's amplitudes and its surface is the CPU's `Ocean::surface` in
//! single precision; [`WaterCascades::read_fields`] reads a cascade back for that comparison.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, GraphBuffer,
    GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    QueueKind, Result, ShaderCompiler, ShaderStage, vk,
};

/// Samples per side of a cascade (`N` in `water.slang`).
pub const WATER_SIZE: u32 = 256;
/// Complex values per sample in the work buffer (`FIELDS`).
const FIELDS: u64 = 4;
/// Threads per side of `evolve` and `derive`'s workgroups.
const GROUP: u32 = 16;

/// Mirrors `WaterPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterPush {
    spectrum: u64,
    work: u64,
    time: f32,
    patch: f32,
    choppiness: f32,
    displacement: u32,
    slopes: u32,
    pad0: u32,
    pad1: [u32; 2],
}

/// One cascade's spectrum and layout, from the CPU.
#[derive(Clone, Debug)]
pub struct WaterCascadeDesc {
    /// The patch's side, metres.
    pub patch: f32,
    /// The horizontal displacement's scale.
    pub choppiness: f32,
    /// Per sample in index order: `h0(k)` (real, imaginary), `ω(k)`, 0
    /// (`forge_procgen::Ocean::gpu_samples`), [`WATER_SIZE`]² of them.
    pub samples: Vec<[f32; 4]>,
}

/// A cascade's fields at one sample, as [`WaterCascades::read_fields`] returns them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaterSample {
    /// The height above the mean sea, metres.
    pub height: f32,
    /// The displacement along +x, metres.
    pub dx: f32,
    /// The displacement along the patch's +y (the world's +z), metres.
    pub dy: f32,
    /// `∂h/∂x`.
    pub slope_x: f32,
    /// `∂h/∂y`.
    pub slope_y: f32,
    /// The Jacobian of the displaced surface (below zero the wave folds over).
    pub jacobian: f32,
}

struct Cascade {
    spectrum: Buffer,
    work: GraphBuffer,
    displacement: GraphImage,
    slopes: GraphImage,
    patch: f32,
    choppiness: f32,
}

/// What this frame's passes write, for the surface pass to read.
#[derive(Clone, Debug)]
pub struct WaterFrame {
    /// Per cascade: the displacement image (x, height, y) and the slopes image (∂h/∂x, ∂h/∂y,
    /// the Jacobian).
    pub cascades: Vec<(ImageHandle, ImageHandle)>,
}

/// The cascades: their spectra, work buffers and images, and the passes that update them.
pub struct WaterCascades {
    evolve: Pipeline,
    rows: Pipeline,
    cols: Pipeline,
    derive: Pipeline,
    cascades: Vec<Cascade>,
}

impl WaterCascades {
    /// Compiles the passes and uploads the cascades' spectra.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        descs: &[WaterCascadeDesc],
    ) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("water.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<WaterPush>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let samples = u64::from(WATER_SIZE * WATER_SIZE);
        let image = |name: &str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: WATER_SIZE,
                    height: WATER_SIZE,
                    format: vk::Format::R16G16B16A16_SFLOAT,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let cascades = descs
            .iter()
            .enumerate()
            .map(|(c, desc)| {
                assert_eq!(
                    desc.samples.len() as u64,
                    samples,
                    "a cascade's spectrum has WATER_SIZE² samples"
                );
                Ok(Cascade {
                    spectrum: device.create_buffer_with_data(
                        &desc.samples,
                        vk::BufferUsageFlags::STORAGE_BUFFER,
                        MemoryCategory::Work,
                        &format!("water spectrum {c}"),
                    )?,
                    work: GraphBuffer::new(device.create_buffer(BufferDesc {
                        size: samples * FIELDS * 8,
                        usage: vk::BufferUsageFlags::STORAGE_BUFFER
                            | vk::BufferUsageFlags::TRANSFER_SRC,
                        location: MemoryLocation::GpuOnly,
                        category: MemoryCategory::Work,
                        name: &format!("water fields {c}"),
                    })?),
                    displacement: image(&format!("water displacement {c}"))?,
                    slopes: image(&format!("water slopes {c}"))?,
                    patch: desc.patch,
                    choppiness: desc.choppiness,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            evolve: compute("evolve_main", "water evolve")?,
            rows: compute("fft_rows_main", "water fft rows")?,
            cols: compute("fft_cols_main", "water fft cols")?,
            derive: compute("derive_main", "water derive")?,
            cascades,
        })
    }

    /// The cascades.
    pub fn len(&self) -> usize {
        self.cascades.len()
    }

    /// Whether there is none.
    pub fn is_empty(&self) -> bool {
        self.cascades.is_empty()
    }

    /// Bytes of the spectra, the work buffers and the images.
    pub fn bytes(&self) -> u64 {
        let texels = u64::from(WATER_SIZE * WATER_SIZE);
        self.cascades
            .iter()
            .map(|c| c.spectrum.size() + c.work.size() + 2 * texels * 8)
            .sum()
    }

    /// Declares this frame's passes, the waves at `time` seconds, on the async compute queue
    /// (they depend on nothing in the frame's geometry), and returns the images they write.
    pub fn update<'f>(&'f self, graph: &mut FrameGraph<'f>, time: f32) -> WaterFrame {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let imported: Vec<_> = self
            .cascades
            .iter()
            .map(|c| {
                (
                    graph.import_buffer(&c.work),
                    graph.import(&c.displacement),
                    graph.import(&c.slopes),
                )
            })
            .collect();
        let push = |c: &Cascade| WaterPush {
            spectrum: c.spectrum.address(),
            work: c.work.address(),
            time,
            patch: c.patch,
            choppiness: c.choppiness,
            displacement: c.displacement.storage(0).0,
            slopes: c.slopes.storage(0).0,
            pad0: 0,
            pad1: [0; 2],
        };
        let groups = WATER_SIZE / GROUP;
        // Each stage over every cascade, so a stage is one zone.
        let stages: [(&'static str, &'f Pipeline, [u32; 3]); 3] = [
            ("water/evolve", &self.evolve, [groups, groups, 1]),
            ("water/fft-rows", &self.rows, [WATER_SIZE, 1, 1]),
            ("water/fft-cols", &self.cols, [WATER_SIZE, 1, 1]),
        ];
        for (label, pipeline, groups) in stages {
            for (cascade, &(work, _, _)) in self.cascades.iter().zip(&imported) {
                let push = push(cascade);
                let access = if label == "water/evolve" {
                    BufferAccess::ShaderWrite(compute)
                } else {
                    BufferAccess::ShaderReadWrite(compute)
                };
                graph
                    .pass(label)
                    .queue(QueueKind::Compute)
                    .buffer(work, access)
                    .run(move |_, commands| {
                        commands.bind_pipeline(pipeline);
                        commands.push_constants(pipeline, &push);
                        commands.dispatch(groups[0], groups[1], groups[2]);
                        Ok(())
                    });
            }
        }
        let derive = &self.derive;
        for (cascade, &(work, displacement, slopes)) in self.cascades.iter().zip(&imported) {
            let push = push(cascade);
            graph
                .pass("water/derive")
                .queue(QueueKind::Compute)
                .buffer(work, BufferAccess::ShaderRead(compute))
                .image(displacement, ImageAccess::StorageWrite(compute))
                .image(slopes, ImageAccess::StorageWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(derive);
                    commands.push_constants(derive, &push);
                    commands.dispatch(groups, groups, 1);
                    Ok(())
                });
        }
        WaterFrame {
            cascades: imported.iter().map(|&(_, d, s)| (d, s)).collect(),
        }
    }

    /// Cascade `index`'s fields as the last submitted frame left them, in sample order.
    /// Waits for the device: for the start-up check and tests.
    pub fn read_fields(&self, device: &Arc<Device>, index: usize) -> Result<Vec<WaterSample>> {
        let work = &self.cascades[index].work;
        let bytes = device.read_back(work, 0, work.size())?;
        let values: &[[[f32; 2]; FIELDS as usize]] = bytemuck::cast_slice(&bytes);
        Ok(values
            .iter()
            .map(|p| {
                let (dxx, dyy, dxy) = (p[2][1], p[3][0], p[3][1]);
                WaterSample {
                    height: p[0][0],
                    dx: p[0][1],
                    dy: p[1][0],
                    slope_x: p[1][1],
                    slope_y: p[2][0],
                    jacobian: (1.0 + dxx) * (1.0 + dyy) - dxy * dxy,
                }
            })
            .collect())
    }
}

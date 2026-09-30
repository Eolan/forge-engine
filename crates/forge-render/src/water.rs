//! The sea's waves on the GPU (issue #105, D-038, `shaders/water.slang`): cascades of FFT
//! waves, each a tiling patch of [`WATER_SIZE`]² samples evolved to the frame's time and
//! transformed on the async compute queue into two persistent images: the displacement (x,
//! height, y) and the slopes with the Jacobian, which marks the whitecaps (Tessendorf 2001).
//! Five graph passes a frame, each over every cascade:
//! - `water/evolve`: the spectrum at the frame's time, the fields' spectra packed two to a
//!   complex value;
//! - `water/fft-rows` and `water/fft-cols`: the inverse transform, radix-2 Stockham in
//!   groupshared memory, a workgroup per line;
//! - `water/derive`: the fields unpacked into the images;
//! - `water/mips`: their mip chains, a level from the one below (the slopes image keeps the
//!   squared slope too, so a mip knows the variance of the slopes it no longer resolves).
//!
//! The spectrum comes from the CPU (`forge_procgen::Ocean::gpu_samples`, uploaded once), so
//! the GPU transforms the CPU's amplitudes and its surface is the CPU's `Ocean::surface` in
//! single precision; [`WaterCascades::read_fields`] reads a cascade back for that comparison.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory,
    MemoryLocation, Pipeline, QueueKind, Result, ShaderCompiler, ShaderStage, TransientDesc,
    VertexPipelineDesc, vk,
};
use glam::{DVec3, Mat4, Vec3};

use crate::aces2::f32_to_f16;
use crate::meshlet::RayRequests;
use crate::sky::SkyFrame;
use crate::taa::HDR_FORMAT;

/// Samples per side of a cascade (`N` in `water.slang`).
pub const WATER_SIZE: u32 = 256;
/// Mip levels of a cascade's images, down to one texel: the mean displacement, slope and
/// squared slope over ever larger areas, for the surface far away.
pub const WATER_MIPS: u32 = WATER_SIZE.ilog2() + 1;
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

/// Mirrors `MipsPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MipsPush {
    displacement: u32,
    slopes: u32,
    displacement_out: u32,
    slopes_out: u32,
    level: u32,
    size: u32,
    pad: [u32; 2],
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
    /// The angular frequency the cascade's energy centres on, rad/s
    /// (`forge_procgen::Ocean::mean_frequency`): the shore damps the cascade by the root of the
    /// TMA factor there (`forge_procgen::tma`, mirrored in `water.slang`).
    pub omega: f32,
    /// The TMA factor at `omega` in the depth the spectrum was made for (1 in deep water): the
    /// shore damps relative to it, so the open sea keeps its waves.
    pub shelf: f32,
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
    omega: f32,
    shelf: f32,
}

/// What this frame's passes write, for the surface pass to read.
#[derive(Clone, Debug)]
pub struct WaterFrame {
    /// Per cascade, with their mips: the displacement image (x, height, y) and the slopes
    /// image (∂h/∂x, ∂h/∂y, the Jacobian, the squared slope).
    pub cascades: Vec<(ImageHandle, ImageHandle)>,
}

/// The cascades: their spectra, work buffers and images, and the passes that update them.
pub struct WaterCascades {
    evolve: Pipeline,
    rows: Pipeline,
    cols: Pipeline,
    derive: Pipeline,
    mips: Pipeline,
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
                    mip_levels: WATER_MIPS,
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
                    omega: desc.omega,
                    shelf: desc.shelf,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            evolve: compute("evolve_main", "water evolve")?,
            rows: compute("fft_rows_main", "water fft rows")?,
            cols: compute("fft_cols_main", "water fft cols")?,
            derive: compute("derive_main", "water derive")?,
            mips: {
                let module = device.create_shader_module(
                    &shaders.compile("water.slang", "mips_main", ShaderStage::Compute)?,
                    "water mips",
                )?;
                let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                    shader: (module, "mips_main"),
                    push_constant_bytes: std::mem::size_of::<MipsPush>() as u32,
                    name: "water mips",
                });
                device.destroy_shader_module(module);
                pipeline?
            },
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
            .map(|c| c.spectrum.size() + c.work.size() + 2 * texels * 8 * 4 / 3)
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
                .image_mip(displacement, 0, ImageAccess::StorageWrite(compute))
                .image_mip(slopes, 0, ImageAccess::StorageWrite(compute))
                .run(move |_, commands| {
                    commands.bind_pipeline(derive);
                    commands.push_constants(derive, &push);
                    commands.dispatch(groups, groups, 1);
                    Ok(())
                });
        }
        // The mips, each level from the one below, every cascade in one pass a level.
        let mips = &self.mips;
        for level in 1..WATER_MIPS {
            let size = WATER_SIZE >> level;
            let mut pass = graph.pass("water/mips").queue(QueueKind::Compute);
            for &(_, displacement, slopes) in &imported {
                pass = pass
                    .image_mip(displacement, level - 1, ImageAccess::Sampled(compute))
                    .image_mip(slopes, level - 1, ImageAccess::Sampled(compute))
                    .image_mip(displacement, level, ImageAccess::StorageWrite(compute))
                    .image_mip(slopes, level, ImageAccess::StorageWrite(compute));
            }
            let images: Vec<_> = imported.iter().map(|&(_, d, s)| (d, s)).collect();
            pass.run(move |resources, commands| {
                commands.bind_pipeline(mips);
                for &(displacement, slopes) in &images {
                    commands.push_constants(
                        mips,
                        &MipsPush {
                            displacement: resources.sampled(displacement).0,
                            slopes: resources.sampled(slopes).0,
                            displacement_out: resources.storage(displacement, level).0,
                            slopes_out: resources.storage(slopes, level).0,
                            level,
                            size,
                            pad: [0; 2],
                        },
                    );
                    commands.dispatch(size.div_ceil(8), size.div_ceil(8), 1);
                }
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

    /// Cascade `index`'s patch side, metres.
    pub fn patch(&self, index: usize) -> f32 {
        self.cascades[index].patch
    }
}

// ------------------------------------------------------------------- surface ---

/// Cascades the surface reads (`WATER_CASCADES` in `water.slang`).
const SURFACE_CASCADES: usize = 3;
/// Clipmap levels the block holds (`WATER_MAX_LEVELS`).
const MAX_LEVELS: usize = 16;
/// Quads a side of a clipmap level: a multiple of 4, so a level's hole falls on the next
/// level's lattice.
const GRID: u32 = 128;
/// Metres between the finest level's vertices.
const FINEST_SPACING: f64 = 0.5;
/// Levels: the finest 64 m across, the coarsest 262 km (the stand-in sea's extent).
const LEVELS: u32 = 13;
/// The surface's second and third targets: per pixel, the mirror ray it asks for (direction,
/// weight) and the shadow ray (the sun's share of the colour, 1).
const REQUEST_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// Mirrors `WaterCascadeView` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuWaterCascadeView {
    offset: [f32; 2],
    inv_patch: f32,
    texel: f32,
    displacement: u32,
    slopes: u32,
    omega: f32,
    shelf: f32,
}

/// Mirrors `WaterSurface` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuWaterSurface {
    view_proj: [f32; 16],
    camera: [f32; 4],
    sun: [f32; 4],
    sun_radiance: [f32; 4],
    absorption: [f32; 4],
    scatter: [f32; 4],
    shore_frame: [f32; 4],
    trains: [[f32; 4]; MAX_TRAINS],
    cascades: [GpuWaterCascadeView; SURFACE_CASCADES],
    levels: [[f32; 4]; MAX_LEVELS],
    level_count: u32,
    grid: u32,
    scene_color: u32,
    scene_depth: u32,
    width: u32,
    height: u32,
    shore: u32,
    shore_texels: u32,
    sky: u64,
    sky_light: u64,
    shore_table: u64,
    shore_bins: u32,
    train_count: u32,
    shore_bin: f32,
    time: f32,
    pad: [f32; 2],
}

const _: () = assert!(std::mem::size_of::<GpuWaterSurface>() == 656);

/// Shore trains the surface draws at most (`WATER_MAX_TRAINS` in `water.slang`).
const MAX_TRAINS: usize = 4;

/// One of the shore's wave trains (`forge_procgen::ShoreTrain`), as the surface draws it.
#[derive(Clone, Copy, Debug)]
pub struct WaterShoreTrain<'a> {
    /// Radians a second.
    pub omega: f32,
    /// The height in deep water, crest to trough, metres.
    pub height: f32,
    /// Per boundary of the shore's bins, out from the shore: the time a crest takes from there
    /// to the shore (seconds) and the shoaling coefficient (`ShoreTrain::table`).
    pub table: &'a [[f32; 2]],
}

/// The sea floor and the coast under the sea (issue #105's shore), on a square grid of samples:
/// what the surface damps its waves by, and the trains of waves that come in to the shore.
#[derive(Clone, Copy, Debug)]
pub struct WaterShore<'a> {
    /// Samples a side.
    pub texels: u32,
    /// Metres between samples.
    pub spacing: f32,
    /// The world x and z of the first sample (the sea's frame, metres).
    pub origin: [f32; 2],
    /// Per sample, row-major along +z: the floor's height, metres (the sea's level at 0, the
    /// land above it).
    pub floor: &'a [f32],
    /// Per sample: the signed distance to the coast, metres, positive inland
    /// (`forge_procgen::coast_distance`).
    pub coast: &'a [f32],
    /// Metres a bin of the trains' tables.
    pub bin: f32,
    /// The trains, [`MAX_TRAINS`] at most, their tables of one length.
    pub trains: &'a [WaterShoreTrain<'a>],
}

/// Mirrors `SurfacePush` and `CopyPush` in `water.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfacePush {
    surface: u64,
    pad: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CopyPush {
    color: u32,
    depth: u32,
    color_out: u32,
    depth_out: u32,
    width: u32,
    height: u32,
    pad: [u32; 2],
}

/// What a frame's sea surface needs besides the cascades and the sky.
#[derive(Clone, Copy, Debug)]
pub struct WaterSurfaceParams {
    /// The drawing camera's view-projection, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// The camera in the sea's frame, metres: the sea's mean level is y = 0.
    pub camera: DVec3,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sun's pre-exposed illuminance on a surface facing it, per channel (the resolve's
    /// sun colour times its illuminance times the exposure).
    pub sun_radiance: Vec3,
    /// The pre-exposed luminance of a unit of sun illuminance (the sky's scale).
    pub sky_scale: f32,
    /// The sea's clock, seconds (the cascades' time): the shore's trains move with it.
    pub time: f32,
}

/// The sea's surface (issue #105, step 2): a clipmap of grids around the camera displaced by
/// the cascades, drawn after the sky's compose (`water/surface`), with the scene it lets
/// through copied first (`water/scene-copy`). It reflects the sky and takes the sun's light,
/// and asks for the rays that bring the scene in ([`RayRequests`]): the mirror rays that put it
/// in the reflection (step 3), and the shadow rays that take the sun away where it shades the
/// water (step 4).
pub struct WaterSurface {
    copy: Pipeline,
    surface: Pipeline,
    blocks: Vec<Buffer>,
    /// Without it the sea is deep everywhere.
    shore: Option<ShoreFields>,
}

/// The shore on the GPU ([`WaterShore`]).
struct ShoreFields {
    /// `RG16F`: the floor's height, the coast distance.
    image: GraphImage,
    /// The first sample's x and z, 1 / spacing, 0.
    frame: [f32; 4],
    texels: u32,
    /// The trains' tables one after the other, `bins` entries each (none without trains).
    tables: Option<Buffer>,
    /// Per train: ω, the deep-water height, a phase offset, 0.
    trains: [[f32; 4]; MAX_TRAINS],
    train_count: u32,
    bins: u32,
    bin: f32,
}

impl WaterSurface {
    /// Compiles the passes and uploads the shore's fields.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        shore: Option<WaterShore<'_>>,
    ) -> Result<Self> {
        let copy_module = device.create_shader_module(
            &shaders.compile("water.slang", "copy_main", ShaderStage::Compute)?,
            "water scene copy",
        )?;
        let copy = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (copy_module, "copy_main"),
            push_constant_bytes: std::mem::size_of::<CopyPush>() as u32,
            name: "water scene copy",
        });
        device.destroy_shader_module(copy_module);
        let vertex = device.create_shader_module(
            &shaders.compile("water.slang", "surface_vert_main", ShaderStage::Vertex)?,
            "water surface vertices",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("water.slang", "surface_frag_main", ShaderStage::Fragment)?,
            "water surface",
        )?;
        let surface = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (vertex, "surface_vert_main"),
            fragment: (fragment, "surface_frag_main"),
            color_formats: &[HDR_FORMAT, REQUEST_FORMAT, REQUEST_FORMAT],
            depth_format: Some(vk::Format::D32_SFLOAT),
            push_constant_bytes: std::mem::size_of::<SurfacePush>() as u32,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: true,
            name: "water surface",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        let blocks = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuWaterSurface>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("water surface {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let shore = shore
            .map(|s| -> Result<_> {
                let count = (s.texels as usize) * (s.texels as usize);
                assert!(
                    s.floor.len() == count && s.coast.len() == count,
                    "the shore's fields hold texels² samples"
                );
                let bytes: Vec<u8> = s
                    .floor
                    .iter()
                    .zip(s.coast)
                    .flat_map(|(&h, &d)| {
                        let [a, b] = f32_to_f16(h).to_le_bytes();
                        let [c, e] = f32_to_f16(d).to_le_bytes();
                        [a, b, c, e]
                    })
                    .collect();
                let image = GraphImage::uploaded(
                    device,
                    ImageDesc {
                        width: s.texels,
                        height: s.texels,
                        format: vk::Format::R16G16_SFLOAT,
                        usage: vk::ImageUsageFlags::SAMPLED,
                        aspect: vk::ImageAspectFlags::COLOR,
                        mip_levels: 1,
                        name: "water shore",
                    },
                    &bytes,
                )?;
                assert!(s.trains.len() <= MAX_TRAINS, "at most {MAX_TRAINS} trains");
                let bins = s.trains.first().map_or(0, |t| t.table.len());
                assert!(
                    s.trains.iter().all(|t| t.table.len() == bins)
                        && (s.trains.is_empty() || bins >= 2),
                    "the trains' tables have one length, two entries at least"
                );
                let table: Vec<[f32; 2]> = s
                    .trains
                    .iter()
                    .flat_map(|t| t.table.iter().copied())
                    .collect();
                let tables = (!table.is_empty())
                    .then(|| {
                        device.create_buffer_with_data(
                            &table,
                            vk::BufferUsageFlags::STORAGE_BUFFER,
                            MemoryCategory::Work,
                            "water shore trains",
                        )
                    })
                    .transpose()?;
                let mut trains = [[0.0; 4]; MAX_TRAINS];
                for (i, (slot, t)) in trains.iter_mut().zip(s.trains).enumerate() {
                    // Each train starts a third of a turn after the one before.
                    *slot = [t.omega, t.height, i as f32 * 2.1, 0.0];
                }
                Ok(ShoreFields {
                    image,
                    frame: [s.origin[0], s.origin[1], 1.0 / s.spacing, 0.0],
                    texels: s.texels,
                    tables,
                    trains,
                    train_count: s.trains.len() as u32,
                    bins: bins as u32,
                    bin: s.bin,
                })
            })
            .transpose()?;
        Ok(Self {
            copy: copy?,
            surface: surface?,
            blocks,
            shore,
        })
    }

    /// Declares the copy of what the water lets through and the surface over `color` and
    /// `depth` (the frame's HDR image and depth, after the sky's compose), from the cascades'
    /// images of this frame (`waves`) and the sky's frame. Returns the rays the surface asks
    /// for, which [`crate::MeshletRenderer::trace_requested`] traces against the scene;
    /// untraced, the water reflects the sky alone and takes the sun everywhere.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        cascades: &'f WaterCascades,
        waves: &WaterFrame,
        sky: &SkyFrame,
        params: WaterSurfaceParams,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
    ) -> RayRequests {
        assert_eq!(
            cascades.len(),
            SURFACE_CASCADES,
            "the surface reads three cascades"
        );
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        let vertex = vk::PipelineStageFlags2::VERTEX_SHADER;
        let transient = |name, format| TransientDesc {
            name,
            width: extent.width,
            height: extent.height,
            format,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let scene_color = graph.transient(transient("water scene color", HDR_FORMAT));
        let scene_depth = graph.transient(transient("water scene depth", vk::Format::R32_SFLOAT));
        let mut request = |name| {
            graph.transient(TransientDesc {
                usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                ..transient(name, REQUEST_FORMAT)
            })
        };
        let mirror_requests = request("water mirror ray requests");
        let sun_requests = request("water shadow ray requests");
        let copy = &self.copy;
        graph
            .pass("water/scene-copy")
            .image(color, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(scene_color, ImageAccess::StorageWrite(compute))
            .image(scene_depth, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(copy);
                commands.push_constants(
                    copy,
                    &CopyPush {
                        color: resources.sampled(color).0,
                        depth: resources.sampled(depth).0,
                        color_out: resources.storage(scene_color, 0).0,
                        depth_out: resources.storage(scene_depth, 0).0,
                        width: extent.width,
                        height: extent.height,
                        pad: [0; 2],
                    },
                );
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });

        // The clipmap: each level centred on the camera snapped to twice its spacing, in f64,
        // then relative to the camera.
        let mut levels = [[0.0_f32; 4]; MAX_LEVELS];
        for (l, level) in levels.iter_mut().enumerate().take(LEVELS as usize) {
            let spacing = FINEST_SPACING * f64::from(1u32 << l);
            let snap = |c: f64| (c / (2.0 * spacing)).floor() * 2.0 * spacing - c;
            *level = [
                snap(params.camera.x) as f32,
                snap(params.camera.z) as f32,
                spacing as f32,
                0.0,
            ];
        }
        let views: Vec<GpuWaterCascadeView> = cascades
            .cascades
            .iter()
            .map(|c| {
                let patch = f64::from(c.patch);
                let offset = |v: f64| (v / patch).rem_euclid(1.0) as f32;
                GpuWaterCascadeView {
                    offset: [offset(params.camera.x), offset(params.camera.z)],
                    inv_patch: 1.0 / c.patch,
                    texel: c.patch / WATER_SIZE as f32,
                    displacement: c.displacement.sampled().0,
                    slopes: c.slopes.sampled().0,
                    omega: c.omega,
                    shelf: c.shelf,
                }
            })
            .collect();
        let block: &'f Buffer = &self.blocks[slot.index];
        let address = block.address();
        let sky_address = sky.address();
        let sky_light = sky.light.address;
        let surface = &self.surface;
        let shore: Option<&'f ShoreFields> = self.shore.as_ref();
        let shore_image = shore.map(|s| graph.import(&s.image));
        let mut pass = graph
            .pass("water/surface")
            .image(color, ImageAccess::ColorAttachment)
            .image(mirror_requests, ImageAccess::ColorAttachment)
            .image(sun_requests, ImageAccess::ColorAttachment)
            .image(depth, ImageAccess::DepthAttachment)
            .image(scene_color, ImageAccess::Sampled(fragment))
            .image(scene_depth, ImageAccess::Sampled(fragment))
            .image(sky.light.table, ImageAccess::Sampled(fragment))
            .image(sky.aerial(), ImageAccess::Sampled(fragment))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(fragment));
        for &(displacement, slopes) in &waves.cascades {
            pass = pass
                .image(displacement, ImageAccess::Sampled(vertex))
                .image(slopes, ImageAccess::Sampled(fragment));
        }
        if let Some(image) = shore_image {
            pass = pass.image(image, ImageAccess::Sampled(vertex | fragment));
        }
        pass.run(move |resources, commands| {
            let mut cascade_views = [GpuWaterCascadeView::zeroed(); SURFACE_CASCADES];
            cascade_views.copy_from_slice(&views);
            block.write(
                0,
                &[GpuWaterSurface {
                    view_proj: params.view_proj.to_cols_array(),
                    camera: [
                        params.camera.x as f32,
                        params.camera.y as f32,
                        params.camera.z as f32,
                        0.0,
                    ],
                    sun: params.sun_dir.normalize_or(Vec3::Y).extend(0.0).to_array(),
                    sun_radiance: params.sun_radiance.extend(params.sky_scale).to_array(),
                    absorption: [0.35, 0.07, 0.05, 0.0],
                    scatter: [0.003, 0.013, 0.016, 0.0],
                    shore_frame: shore.map_or([0.0; 4], |s| s.frame),
                    trains: shore.map_or([[0.0; 4]; MAX_TRAINS], |s| s.trains),
                    cascades: cascade_views,
                    levels,
                    level_count: LEVELS,
                    grid: GRID,
                    scene_color: resources.sampled(scene_color).0,
                    scene_depth: resources.sampled(scene_depth).0,
                    width: extent.width,
                    height: extent.height,
                    shore: shore_image.map_or(u32::MAX, |image| resources.sampled(image).0),
                    shore_texels: shore.map_or(0, |s| s.texels),
                    sky: sky_address,
                    sky_light,
                    shore_table: shore
                        .and_then(|s| s.tables.as_ref())
                        .map_or(0, |b| b.address()),
                    shore_bins: shore.map_or(0, |s| s.bins),
                    train_count: shore.map_or(0, |s| s.train_count),
                    shore_bin: shore.map_or(1.0, |s| s.bin),
                    time: params.time,
                    pad: [0.0; 2],
                }],
            );
            // The requests start at zero: no ray where the water is not drawn.
            let cleared = |image| {
                vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(image))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .clear_value(vk::ClearValue::default())
            };
            let attachment = [
                vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(color))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE),
                cleared(mirror_requests),
                cleared(sun_requests),
            ];
            let depth_attachment = vk::RenderingAttachmentInfo::default()
                .image_view(resources.view(depth))
                .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE);
            let info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                })
                .layer_count(1)
                .color_attachments(&attachment)
                .depth_attachment(&depth_attachment);
            commands.begin_rendering(&info);
            commands.bind_pipeline(surface);
            commands.set_viewport_full(extent);
            commands.push_constants(
                surface,
                &SurfacePush {
                    surface: address,
                    pad: [0; 2],
                },
            );
            commands.draw(LEVELS * GRID * GRID * 6, 1);
            commands.end_rendering();
            Ok(())
        });
        RayRequests {
            mirror: mirror_requests,
            sun: sun_requests,
            depth,
            view_proj: params.view_proj,
        }
    }
}

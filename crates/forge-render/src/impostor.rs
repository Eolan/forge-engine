//! Impostors (D-075 step 4, #302): a tree far off drawn as one quad showing it baked from the
//! nearest of a hemisphere's directions (`research/forests-and-grass.md` §8.1, R. Brucks,
//! "Octahedral Impostors", 2018).
//!
//! - **The layout:** the upper hemisphere (+Y) unfolded into a square, a diamond turned to fill
//!   it: its middle straight up, its edges the horizon. [`FRAMES`] × [`FRAMES`] frames, each
//!   baked from the direction at its middle ([`frame_direction`]); a view below the horizon is
//!   taken at the horizon.
//! - **The blend:** a view's point in the layout falls in the triangle of three frames' middles
//!   ([`frames_for`]), which are blended by its barycentric weights, so a turning camera never
//!   jumps from one frame to the next.
//! - **The bake** ([`ImpostorBaker`]), before the run's first counted frame: each frame drawn by
//!   the meshlet renderer unlit, its base colour and its normals with their depth; then the
//!   occlusion, every frame read as a shadow map from each texel's point.
//! - **The draw** ([`Impostors`]), after the resolve: where an instance's meshes fade, its
//!   impostor takes the pixels they leave, its depth where the bake saw the leaves, lit as the
//!   resolve lights a rough row (`shaders/impostor_draw.slang`).

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_core::material::MaterialId;
use forge_geom::meshlet::MeshletMesh;
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle,
    MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage,
    VertexPipelineDesc, vk,
};
use glam::{Mat4, Quat, UVec2, Vec2, Vec3};

use crate::CellPos;
use crate::material::GpuMaterial;
use crate::meshlet::{
    AmbientLight, CullCamera, CullFlags, DrawParams, InstanceOcclusion, MeshletRenderer,
    MeshletScene, MeshletSceneBuilder, SwRaster,
};
use crate::sky::SkyLight;
use crate::taa::{HDR_FORMAT, MOTION_FORMAT};

/// Frames a side of an impostor's layout (D-075's question 7: 8 × 8 frames).
pub const FRAMES: u32 = 8;
/// Texels a side of a frame: 64, where D-075's question 7 had 128. An impostor begins under
/// its fade's size and band (40 px across by default, 50 at most), so a frame still gives each
/// pixel more than a texel, and an impostor for each age of a species (its proportions, not the
/// oldest's stretched) costs no more than one for each species at 128.
pub const FRAME_TEXELS: u32 = 64;
/// Texels a side of an atlas: every frame's.
pub const ATLAS_TEXELS: u32 = FRAMES * FRAME_TEXELS;
/// Texels a side of a frame's render per atlas texel: an edge's coverage from 16 samples.
const SAMPLES: u32 = 4;
/// Texels a side of a frame's render.
const RENDER_TEXELS: u32 = FRAME_TEXELS * SAMPLES;
/// How far the bake's camera stands from the mesh's middle, in radii of its sphere: near
/// enough to a parallel view that the frames show it without perspective.
const CAMERA_DISTANCE: f32 = 1000.0;
/// The atlases a bake makes: the base colour and its coverage, the normals and the depth, then
/// the occlusion.
const ATLASES: usize = 3;
/// The atlases drawn frame by frame; the occlusion is made from them.
const RENDERED: u32 = 2;
/// Each atlas's format.
const ATLAS_FORMATS: [vk::Format; ATLASES] = [
    vk::Format::R16G16B16A16_SFLOAT,
    vk::Format::R16G16B16A16_SFLOAT,
    vk::Format::R16G16_SFLOAT,
];
/// An atlas's mips: down to frames of a texel.
pub const ATLAS_MIPS: u32 = FRAME_TEXELS.ilog2() + 1;

/// The unit direction, from the impostor's middle towards the eye, that the point `uv` of the
/// layout stands for (0 to 1 both ways).
pub fn direction(uv: Vec2) -> Vec3 {
    let p = uv * 2.0 - Vec2::ONE;
    let (x, z) = (0.5 * (p.x + p.y), 0.5 * (p.x - p.y));
    Vec3::new(x, (1.0 - x.abs() - z.abs()).max(0.0), z).normalize()
}

/// Where direction `d` (towards the eye) falls in the layout, 0 to 1 both ways: the inverse of
/// [`direction`], a direction below the horizon taken at the horizon.
pub fn layout_point(d: Vec3) -> Vec2 {
    let d = Vec3::new(d.x, d.y.max(0.0), d.z);
    let d = d / (d.x.abs() + d.y + d.z.abs()).max(1e-12);
    Vec2::new(d.x + d.z, d.x - d.z) * 0.5 + Vec2::splat(0.5)
}

/// The direction frame `frame` (column, row) is baked from: its middle in the layout.
pub fn frame_direction(frame: UVec2) -> Vec3 {
    direction((frame.as_vec2() + 0.5) / FRAMES as f32)
}

/// The three frames a view from direction `d` blends and their weights (summing to 1): the
/// corners of the triangle of frame middles its layout point falls in, half of the square of
/// four, split along its rising diagonal; at the layout's edge, the nearest frames.
pub fn frames_for(d: Vec3) -> [(UVec2, f32); 3] {
    let grid = layout_point(d) * FRAMES as f32 - 0.5;
    let last = (FRAMES - 1) as f32;
    let corner = grid.floor().clamp(Vec2::ZERO, Vec2::splat(last - 1.0));
    let t = (grid - corner).clamp(Vec2::ZERO, Vec2::ONE);
    let at = |dx: u32, dy: u32| corner.as_uvec2() + UVec2::new(dx, dy);
    if t.x + t.y <= 1.0 {
        [
            (at(0, 0), 1.0 - t.x - t.y),
            (at(1, 0), t.x),
            (at(0, 1), t.y),
        ]
    } else {
        [
            (at(1, 1), t.x + t.y - 1.0),
            (at(0, 1), 1.0 - t.x),
            (at(1, 0), 1.0 - t.y),
        ]
    }
}

/// The axes of frame `direction`'s picture in the scene frame: its right and its up, the view
/// along `-direction`. The bake and the impostor's shading agree on them.
pub fn frame_axes(direction: Vec3) -> (Vec3, Vec3) {
    let right = Vec3::Y.cross(direction);
    let right = if right.length_squared() > 1e-8 {
        right.normalize()
    } else {
        Vec3::X
    };
    (right, direction.cross(right))
}

/// What an impostor is baked from: a mesh, the material rows it is drawn with (the scene's,
/// [`crate::material::gpu_rows`], whose textures the caller keeps alive) and its first row.
pub struct BakeSource {
    /// The mesh, its full detail drawn.
    pub mesh: MeshletMesh,
    /// The rows.
    pub rows: Vec<GpuMaterial>,
    /// The mesh's first row among them.
    pub material: MaterialId,
    /// Its leaves' row among them: the sheen and the light through that its impostor keeps.
    pub leaves: MaterialId,
}

/// An impostor's atlases, read back: [`ATLAS_TEXELS`]² texels each, rows top down, frame
/// (column, row) at its [`FRAME_TEXELS`]² square.
pub struct BakedAtlases {
    /// The base colour (linear) premultiplied by the share of the texel the mesh covers, and
    /// that share.
    pub albedo: Vec<[f32; 4]>,
    /// The normal in the mesh's frame (× 0.5 + 0.5) and the depth: the surface's distance from
    /// the mesh's middle towards the frame's camera, in radii of its sphere; both premultiplied
    /// by the coverage (the base colour's alpha).
    pub normal: Vec<[f32; 4]>,
    /// The share of the sky the point sees past the mesh itself, premultiplied by the coverage.
    pub occlusion: Vec<f32>,
}

/// The downsample's push constants (`Push` in `shaders/impostor.slang`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DownsamplePush {
    source: u32,
    atlas: u32,
    tile: [u32; 2],
    samples: u32,
    normals: u32,
    depth: u32,
    pad: u32,
}

/// The occlusion's push constants (`OcclusionPush` in `shaders/impostor.slang`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OcclusionPush {
    albedo: u32,
    normal: u32,
    occlusion: u32,
    pad: u32,
}

/// A storage, sampled and copied half-float image of `format`, `side` texels square with `mips`
/// levels: a bake's render or one of its atlases.
fn bake_image(
    device: &Arc<Device>,
    name: &'static str,
    side: u32,
    mips: u32,
    format: vk::Format,
) -> Result<GraphImage> {
    GraphImage::new(
        device,
        ImageDesc {
            width: side,
            height: side,
            format,
            usage: vk::ImageUsageFlags::STORAGE
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: mips,
            name,
        },
    )
}

/// Declares the passes "impostor/mips" of `image`: each level the average of the finer one
/// (premultiplied texels), by `pipeline` (the RGBA or the RG one).
fn declare_mips<'f>(graph: &mut FrameGraph<'f>, image: ImageHandle, pipeline: &'f Pipeline) {
    for level in 1..ATLAS_MIPS {
        let side = ATLAS_TEXELS >> level;
        graph
            .pass("impostor/mips")
            .image(
                image,
                ImageAccess::StorageReadWrite(vk::PipelineStageFlags2::COMPUTE_SHADER),
            )
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &DownsamplePush {
                        source: resources.storage(image, level - 1).0,
                        atlas: resources.storage(image, level).0,
                        tile: [side, side],
                        samples: 2,
                        normals: 0,
                        depth: 0,
                        pad: 0,
                    },
                );
                commands.dispatch(side.div_ceil(8), side.div_ceil(8), 1);
                Ok(())
            });
    }
}

/// What every bake shares: the renderer (its resolve compiled from `meshlet_bake.slang` once),
/// the render a frame is drawn into, the downsample, the mips (RGBA and RG) and the occlusion.
struct BakeTools {
    renderer: MeshletRenderer,
    render: GraphImage,
    downsample: Pipeline,
    mips: Pipeline,
    mips_rg: Pipeline,
    occlusion: Pipeline,
}

/// The bakes of a scene's impostors (D-075 step 4), one after another, one of their frames
/// each app frame: [`ImpostorBaker::record`] declares the current bake's part of the frame and
/// [`ImpostorBaker::poll`] hands each bake back once its atlases are read back.
pub struct ImpostorBaker {
    tools: BakeTools,
    /// The sources still to bake, the next last.
    queue: Vec<(String, BakeSource)>,
    current: Option<(String, ImpostorBake)>,
}

impl ImpostorBaker {
    /// Prepares the bakes of `sources`, named, in order.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        mut sources: Vec<(String, BakeSource)>,
    ) -> Result<Self> {
        let extent = vk::Extent2D {
            width: RENDER_TEXELS,
            height: RENDER_TEXELS,
        };
        let compute = |entry: &str, name: &str, push: usize| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("impostor.slang", entry, ShaderStage::Compute)?,
                entry,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: push as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let push = std::mem::size_of::<DownsamplePush>();
        sources.reverse();
        Ok(Self {
            tools: BakeTools {
                renderer: MeshletRenderer::for_bake(device, shaders, extent)?,
                render: bake_image(
                    device,
                    "impostor render",
                    RENDER_TEXELS,
                    1,
                    ATLAS_FORMATS[0],
                )?,
                downsample: compute("impostor_downsample_main", "impostor downsample", push)?,
                mips: compute("impostor_mip_main", "impostor mips", push)?,
                mips_rg: compute("impostor_mip_rg_main", "impostor mips (rg)", push)?,
                occlusion: compute(
                    "impostor_occlusion_main",
                    "impostor occlusion",
                    std::mem::size_of::<OcclusionPush>(),
                )?,
            },
            queue: sources,
            current: None,
        })
    }

    /// Whether every bake was handed back.
    pub fn is_done(&self) -> bool {
        self.current.is_none() && self.queue.is_empty()
    }

    /// At a frame's start, before [`Self::record`]: the bake whose atlases were read back, if
    /// one was, with its name; the next begun when none is under way.
    pub fn poll(&mut self, device: &Arc<Device>) -> Result<Option<(String, ImpostorBake)>> {
        let finished = if self
            .current
            .as_ref()
            .is_some_and(|(_, bake)| bake.read.get())
        {
            self.current.take()
        } else {
            None
        };
        if self.current.is_none()
            && let Some((name, source)) = self.queue.pop()
        {
            self.current = Some((name, ImpostorBake::new(device, &source)?));
        }
        Ok(finished)
    }

    /// Declares this frame's part of the current bake into `graph`, if one is under way.
    pub fn record<'f>(&'f self, graph: &mut FrameGraph<'f>, slot: FrameSlot) -> Result<()> {
        match &self.current {
            Some((_, bake)) => bake.record(&self.tools, graph, slot),
            None => Ok(()),
        }
    }
}

/// An impostor's bake on the GPU (D-075 step 4), driven by an [`ImpostorBaker`]: a scene of the
/// one mesh, drawn from each frame's direction ([`frame_direction`]) with a camera
/// [`CAMERA_DISTANCE`] radii off, its sphere filling the picture, at full detail; the resolve
/// writes its base colour or its normal unlit ([`CullFlags::BAKE_ALBEDO`],
/// [`CullFlags::BAKE_NORMAL`]), and the downsample takes the render into the frame's square of
/// the atlas, the normals with the render's depth. Then the two atlases' mips are made, the
/// occlusion from them and its mips, and the three are read back.
pub struct ImpostorBake {
    scene: MeshletScene,
    centre: Vec3,
    radius: f32,
    /// Its leaves' row's sheen and two colours (`GpuMaterial::sheen`, `GpuMaterial::tones`).
    sheen: ([f32; 3], f32, f32, f32),
    tones: [[f32; 3]; 2],
    atlases: [GraphImage; ATLASES],
    readback: [GraphBuffer; ATLASES],
    /// The next frame to bake, the base colour's frames first.
    next: Cell<u32>,
    /// The frame slot that copied the atlases out, read once it comes round again.
    copied: Cell<Option<usize>>,
    baked: RefCell<Option<BakedAtlases>>,
    /// Whether the atlases were read back.
    read: Cell<bool>,
}

impl ImpostorBake {
    fn new(device: &Arc<Device>, source: &BakeSource) -> Result<Self> {
        let mut builder = MeshletSceneBuilder::new();
        let mesh = builder.add_mesh(&source.mesh);
        builder.set_material_rows(source.rows.clone());
        builder.add_instance_with_material(mesh, Mat4::IDENTITY, source.material);
        let scene = builder.build(device)?;
        let readback = |name: &'static str, texel_bytes: u64| {
            device
                .create_buffer(BufferDesc {
                    size: u64::from(ATLAS_TEXELS) * u64::from(ATLAS_TEXELS) * texel_bytes,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name,
                })
                .map(GraphBuffer::new)
        };
        Ok(Self {
            scene,
            centre: Vec3::from_array(source.mesh.center),
            radius: source.mesh.radius.max(1e-3),
            sheen: source.rows[source.leaves.0 as usize].sheen(),
            tones: source.rows[source.leaves.0 as usize].tones(),
            atlases: [
                bake_image(
                    device,
                    "impostor albedo",
                    ATLAS_TEXELS,
                    ATLAS_MIPS,
                    ATLAS_FORMATS[0],
                )?,
                bake_image(
                    device,
                    "impostor normal",
                    ATLAS_TEXELS,
                    ATLAS_MIPS,
                    ATLAS_FORMATS[1],
                )?,
                bake_image(
                    device,
                    "impostor occlusion",
                    ATLAS_TEXELS,
                    ATLAS_MIPS,
                    ATLAS_FORMATS[2],
                )?,
            ],
            readback: [
                readback("impostor albedo read back", 8)?,
                readback("impostor normal read back", 8)?,
                readback("impostor occlusion read back", 4)?,
            ],
            next: Cell::new(0),
            copied: Cell::new(None),
            baked: RefCell::new(None),
            read: Cell::new(false),
        })
    }

    /// Takes the atlases, once read back.
    pub fn take(&self) -> Option<BakedAtlases> {
        self.baked.borrow_mut().take()
    }

    /// The bake, once read back, as a kind of impostor to draw ([`Impostors`]), standing in
    /// for meshes that fade over `fade_px` and `fade_band` (`MeshletSceneBuilder::set_fade`).
    pub fn into_kind(self, fade_px: f32, fade_band: f32) -> ImpostorKind {
        assert!(self.read.get(), "an impostor is drawn once baked");
        ImpostorKind {
            atlases: self.atlases,
            radius: self.radius,
            sheen: self.sheen,
            tones: self.tones,
            fade_px,
            fade_band,
        }
    }

    /// Declares this frame's part of the bake into `graph`: the next frame drawn into its
    /// square, or the mips made and the atlases copied out after the last, or nothing while the
    /// copy is in flight.
    fn record<'f>(
        &'f self,
        tools: &'f BakeTools,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
    ) -> Result<()> {
        let total = RENDERED * FRAMES * FRAMES;
        if let Some(copied) = self.copied.get() {
            // The slot comes round once its frame is done: the copies have landed.
            if copied == slot.index && !self.read.get() {
                self.read.set(true);
                let halves = |buffer: &GraphBuffer, channels: u32| {
                    let mut halves = vec![0u16; (ATLAS_TEXELS * ATLAS_TEXELS * channels) as usize];
                    buffer.read(0, &mut halves);
                    halves
                };
                let rgba = |buffer: &GraphBuffer| {
                    halves(buffer, 4)
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|t| t.map(half_to_f32))
                        .collect::<Vec<_>>()
                };
                *self.baked.borrow_mut() = Some(BakedAtlases {
                    albedo: rgba(&self.readback[0]),
                    normal: rgba(&self.readback[1]),
                    occlusion: halves(&self.readback[2], 2)
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|t| half_to_f32(t[0]))
                        .collect(),
                });
            }
            return Ok(());
        }
        if self.next.get() == total {
            self.copied.set(Some(slot.index));
            let extent = vk::Extent2D {
                width: ATLAS_TEXELS,
                height: ATLAS_TEXELS,
            };
            let images = self.atlases.each_ref().map(|atlas| graph.import(atlas));
            // The base colour's and the normals' mips first: the occlusion reads them.
            declare_mips(graph, images[0], &tools.mips);
            declare_mips(graph, images[1], &tools.mips);
            let occlusion = &tools.occlusion;
            let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
            graph
                .pass("impostor/occlusion")
                .image(images[0], ImageAccess::Sampled(compute))
                .image(images[1], ImageAccess::Sampled(compute))
                .image(images[2], ImageAccess::StorageWrite(compute))
                .run(move |resources, commands| {
                    commands.bind_pipeline(occlusion);
                    commands.push_constants(
                        occlusion,
                        &OcclusionPush {
                            albedo: resources.sampled(images[0]).0,
                            normal: resources.sampled(images[1]).0,
                            occlusion: resources.storage(images[2], 0).0,
                            pad: 0,
                        },
                    );
                    commands.dispatch(ATLAS_TEXELS / 8, ATLAS_TEXELS / 8, 1);
                    Ok(())
                });
            declare_mips(graph, images[2], &tools.mips_rg);
            for (&image, readback) in images.iter().zip(&self.readback) {
                let buffer = graph.import_buffer(readback);
                graph
                    .pass("impostor/read back")
                    .image(image, ImageAccess::TransferSrc)
                    .buffer(buffer, BufferAccess::TransferDst)
                    .run(move |resources, commands| {
                        commands.copy_image_to_buffer(resources.image(image).raw, extent, readback);
                        Ok(())
                    });
                graph
                    .pass("impostor/read back")
                    .buffer(buffer, BufferAccess::HostRead)
                    .run(|_, _| Ok(()));
            }
            return Ok(());
        }
        let k = self.next.get();
        self.next.set(k + 1);
        let atlas_index = (k / (FRAMES * FRAMES)) as usize;
        let frame = UVec2::new(k % FRAMES, (k / FRAMES) % FRAMES);
        let direction = frame_direction(frame);
        let (right, up) = frame_axes(direction);
        let distance = CAMERA_DISTANCE * self.radius;
        let eye = self.centre + direction * distance;
        // Camera-relative → view: the rotation alone (D-004), looking along -direction.
        let view = Mat4::from_cols(
            right.extend(0.0),
            up.extend(0.0),
            direction.extend(0.0),
            glam::Vec4::W,
        )
        .transpose();
        let near = distance - 2.0 * self.radius;
        let fov = 2.0 * (self.radius / distance).atan();
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 1.0, near);
        let cull = CullCamera::new(view, proj, CellPos::ORIGIN.offset(eye), near);
        let mode = if atlas_index == 0 {
            CullFlags::BAKE_ALBEDO
        } else {
            CullFlags::BAKE_NORMAL
        };
        let extent = vk::Extent2D {
            width: RENDER_TEXELS,
            height: RENDER_TEXELS,
        };
        let targets = tools.renderer.draw(
            graph,
            slot,
            DrawParams {
                scene: &self.scene,
                view_proj: cull.view_proj,
                cull,
                draw_jitter: Vec2::ZERO,
                lod_threshold_px: 1.0,
                // Full detail (no LOD), no occlusion: the one mesh.
                flags: CullFlags(CullFlags::FRUSTUM | CullFlags::CONE | mode),
                extent,
                wireframe: false,
                // The resolve's scale back to the albedo itself.
                exposure: std::f32::consts::PI / crate::starfield::SUN_ILLUMINANCE_1AU,
                sw_raster: SwRaster::Off,
                instance_occlusion: InstanceOcclusion::Off,
                instance_cells: false,
                sw_raster_area: crate::meshlet::SW_RASTER_DEFAULT_AREA,
            },
        )?;
        let (render, depth) = (graph.import(&tools.render), targets.depth);
        tools.renderer.resolve(
            graph,
            slot,
            targets,
            render,
            extent,
            Some([0.0; 4]),
            AmbientLight::default(),
        );
        let atlas = graph.import(&self.atlases[atlas_index]);
        let downsample = &tools.downsample;
        let tile = frame * FRAME_TEXELS;
        graph
            .pass("impostor/downsample")
            .image(
                render,
                ImageAccess::StorageRead(vk::PipelineStageFlags2::COMPUTE_SHADER),
            )
            .image(
                depth,
                ImageAccess::Sampled(vk::PipelineStageFlags2::COMPUTE_SHADER),
            )
            .image(
                atlas,
                ImageAccess::StorageWrite(vk::PipelineStageFlags2::COMPUTE_SHADER),
            )
            .run(move |resources, commands| {
                commands.bind_pipeline(downsample);
                commands.push_constants(
                    downsample,
                    &DownsamplePush {
                        source: resources.storage(render, 0).0,
                        atlas: resources.storage(atlas, 0).0,
                        tile: tile.to_array(),
                        samples: SAMPLES,
                        normals: u32::from(atlas_index == 1),
                        depth: resources.sampled(depth).0,
                        pad: 0,
                    },
                );
                commands.dispatch(FRAME_TEXELS / 8, FRAME_TEXELS / 8, 1);
                Ok(())
            });
        Ok(())
    }

    /// Writes the atlases as PNG images to `dir`, for review: `{name}-albedo.png` in sRGB with
    /// its coverage as alpha, `{name}-normal.png`, `{name}-depth.png` (grey, the middle's plane
    /// mid-grey, a radius towards the camera white) and `{name}-occlusion.png` (white: all the
    /// sky seen), with the same alpha.
    pub fn write_png(
        atlases: &BakedAtlases,
        dir: &std::path::Path,
        name: &str,
    ) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let srgb = |c: f32| {
            let c = c.clamp(0.0, 1.0);
            let s = if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (s * 255.0 + 0.5) as u8
        };
        let unorm = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        // Stored premultiplied by the coverage: back to straight values for the images.
        let save = |path: std::path::PathBuf,
                    texel: &dyn Fn([f32; 4], [f32; 4], f32) -> [u8; 3]| {
            let bytes: Vec<u8> = atlases
                .albedo
                .iter()
                .zip(&atlases.normal)
                .zip(&atlases.occlusion)
                .flat_map(|((&albedo, &normal), &occlusion)| {
                    let [r, g, b] = texel(albedo, normal, occlusion);
                    [r, g, b, unorm(albedo[3])]
                })
                .collect();
            image::save_buffer(
                &path,
                &bytes,
                ATLAS_TEXELS,
                ATLAS_TEXELS,
                image::ColorType::Rgba8,
            )
            .map_err(std::io::Error::other)
        };
        let straight = |t: [f32; 4], a: f32| t.map(|c| c / a.max(1e-4));
        save(dir.join(format!("{name}-albedo.png")), &|a, _, _| {
            let t = straight(a, a[3]);
            [srgb(t[0]), srgb(t[1]), srgb(t[2])]
        })?;
        save(dir.join(format!("{name}-normal.png")), &|a, n, _| {
            let t = straight(n, a[3]);
            [unorm(t[0]), unorm(t[1]), unorm(t[2])]
        })?;
        save(dir.join(format!("{name}-depth.png")), &|a, n, _| {
            let d = unorm(straight(n, a[3])[3] * 0.5 + 0.5);
            [d, d, d]
        })?;
        save(dir.join(format!("{name}-occlusion.png")), &|a, _, o| {
            let o = unorm(o / a[3].max(1e-4));
            [o, o, o]
        })
    }
}

/// A baked impostor on the GPU (D-075 step 4): its atlases, sampled with their mips, the radius
/// of the sphere they were baked around, and the fade of the meshes it stands in for.
pub struct ImpostorKind {
    atlases: [GraphImage; ATLASES],
    radius: f32,
    sheen: ([f32; 3], f32, f32, f32),
    tones: [[f32; 3]; 2],
    fade_px: f32,
    fade_band: f32,
}

/// One plant drawn as an impostor where its meshes fade (D-075 step 4).
#[derive(Clone, Copy, Debug)]
pub struct ImpostorInstance {
    /// Its kind: the index of its source among those baked.
    pub kind: u32,
    /// Its instance's culling sphere's middle, as the scene has it: where its fade is
    /// measured from.
    pub position: CellPos,
    /// From there to the middle of its kind's baked sphere, placed as the instance places its
    /// mesh, metres.
    pub middle: Vec3,
    /// The instance's turn.
    pub rotation: Quat,
    /// The instance's (uniform) scale.
    pub scale: f32,
    /// The instance's culling sphere's radius in the scene: what its meshes' fade measures.
    pub cull_radius: f32,
    /// The instance's id in the scene, which offsets its fade's crossing as the culls do.
    pub id: u32,
    /// The crossing its meshes' ladder makes before their fade (`set_ladder`'s size on screen
    /// and band), if one: its impostor waits for it to end.
    pub ladder: Option<(f32, f32)>,
}

/// A scene's impostors (D-075 step 4): the meshes to bake and the plants drawn with them.
pub struct ImpostorSet {
    /// The kinds' sources, each named for its atlases' files.
    pub sources: Vec<(String, BakeSource)>,
    /// The plants drawn as impostors where their meshes fade (none: the bakes alone).
    pub instances: Vec<ImpostorInstance>,
    /// Their meshes' fade ([`MeshletSceneBuilder::set_fade`]): the size on screen, output
    /// pixels across, at which it is half done.
    pub fade_px: f32,
    /// The fade's band.
    pub fade_band: f32,
}

/// What the impostors' draw takes of the frame.
#[derive(Clone, Copy, Debug)]
pub struct ImpostorView {
    /// Camera-relative → clip, jittered as the scene was drawn.
    pub view_proj: Mat4,
    /// The previous frame's unjittered clip from this one's (`TaaFrame`).
    pub previous_from_current: Mat4,
    /// This frame's jitter, pixels.
    pub jitter: Vec2,
    /// Where the camera stands.
    pub camera: CellPos,
    /// The projection's y scale, as the culls measure sizes on screen.
    pub p11: f32,
    /// The near plane, metres.
    pub near: f32,
    /// The renderer's textures' LOD bias, stops (`MeshletRenderer::texture_lod_bias`).
    pub texture_lod_bias: f32,
    /// The renderer's noise frame (`MeshletRenderer::noise_frame`).
    pub noise_frame: u32,
    /// Towards the sun.
    pub sun_dir: Vec3,
    /// The sunlight's colour at the scene.
    pub sun_color: Vec3,
    /// The sun's illuminance, lux.
    pub sun_illuminance: f32,
    /// The frame's pre-exposure.
    pub exposure: f32,
    /// The sky's irradiance, when the sky lights the scene.
    pub sky: Option<SkyLight>,
    /// The camera in the scene's frame, where the rays are traced.
    pub camera_in_scene: Vec3,
    /// The scene's rays' structure (`SceneRays::tlas_address`), 0 for none: the sun's shadow
    /// rays meet its far shadows' world (D-075 step 3).
    pub tlas: u64,
    /// Its storage, in a scene whose static instances change (`MoversFrame::static_tlas`, #323).
    pub static_tlas: Option<BufferHandle>,
    /// The render's size.
    pub extent: vk::Extent2D,
}

/// Mirrors `Impostor` in `shaders/impostor_draw.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuImpostor {
    cell: [i32; 3],
    kind: u32,
    local: [f32; 3],
    scale: f32,
    rotation: [f32; 4],
    middle: [f32; 3],
    cull_radius: f32,
    id: u32,
    ladder_px: f32,
    ladder_band: f32,
    pad: u32,
}

/// Mirrors `Kind` in `shaders/impostor_draw.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuKind {
    albedo: u32,
    normal: u32,
    occlusion: u32,
    fade_px: f32,
    fade_band: f32,
    radius: f32,
    reflectance: f32,
    specular: f32,
    through: [f32; 3],
    power: f32,
    tone_a: [f32; 3],
    pad_a: u32,
    tone_b: [f32; 3],
    pad_b: u32,
}

/// Mirrors `Frame` in `shaders/impostor_draw.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuImpostorFrame {
    view_proj: [f32; 16],
    previous_from_current: [f32; 16],
    camera_cell: [i32; 3],
    noise_frame: u32,
    camera_local: [f32; 3],
    light_scale: f32,
    sun_dir: [f32; 3],
    lod_k: f32,
    sun_color: [f32; 3],
    znear: f32,
    camera_in_scene: [f32; 3],
    pad: u32,
    jitter: [f32; 2],
    size: [f32; 2],
    instances: u64,
    kinds: u64,
    sky: u64,
    tlas: u64,
}

const _: () = assert!(std::mem::size_of::<GpuImpostor>() == 80);
const _: () = assert!(std::mem::size_of::<GpuKind>() == 80);
const _: () = assert!(std::mem::offset_of!(GpuImpostorFrame, instances) == 224);

/// The impostors drawn (D-075 step 4): far plants as quads facing the camera, each showing its
/// kind's atlases blended from the three frames around the view, drawn after the resolve and
/// before the sky's compose over the scene's depth (`shaders/impostor_draw.slang`). Where an
/// instance's meshes fade, its impostor takes the pixels they leave: the same crossing, the
/// complement of their noise. Every instance is drawn each frame; those not crossing are moved
/// off screen by the vertex shader.
pub struct Impostors {
    kinds: Vec<ImpostorKind>,
    kind_table: Buffer,
    instances: Buffer,
    count: u32,
    frames: Vec<Buffer>,
    /// Writes the pixels' motion too, when the motion vectors were made before.
    with_motion: Pipeline,
    color_only: Pipeline,
}

impl Impostors {
    /// The impostors of `instances`, of `kinds`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        kinds: Vec<ImpostorKind>,
        instances: &[ImpostorInstance],
    ) -> Result<Self> {
        let table: Vec<GpuKind> = kinds
            .iter()
            .map(|k| GpuKind {
                albedo: k.atlases[0].sampled().0,
                normal: k.atlases[1].sampled().0,
                occlusion: k.atlases[2].sampled().0,
                fade_px: k.fade_px,
                fade_band: k.fade_band,
                radius: k.radius,
                reflectance: k.sheen.1,
                specular: k.sheen.2,
                through: k.sheen.0,
                power: k.sheen.3,
                tone_a: k.tones[0],
                pad_a: 0,
                tone_b: k.tones[1],
                pad_b: 0,
            })
            .collect();
        let gpu: Vec<GpuImpostor> = instances
            .iter()
            .map(|i| {
                let at = i.position.normalized();
                GpuImpostor {
                    cell: at.cell.to_array(),
                    kind: i.kind,
                    local: at.local.to_array(),
                    scale: i.scale,
                    rotation: i.rotation.to_array(),
                    middle: i.middle.to_array(),
                    cull_radius: i.cull_radius,
                    id: i.id,
                    ladder_px: i.ladder.map_or(0.0, |l| l.0),
                    ladder_band: i.ladder.map_or(0.0, |l| l.1),
                    pad: 0,
                }
            })
            .collect();
        let table_of = |bytes: usize, name: &str| {
            device.create_buffer(BufferDesc {
                size: bytes.max(16) as u64,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::CpuToGpu,
                category: MemoryCategory::Work,
                name,
            })
        };
        let kind_table = table_of(std::mem::size_of_val(table.as_slice()), "impostor kinds")?;
        if !table.is_empty() {
            kind_table.write(0, &table);
        }
        let instance_table = table_of(std::mem::size_of_val(gpu.as_slice()), "impostors")?;
        if !gpu.is_empty() {
            instance_table.write(0, &gpu);
        }
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuImpostorFrame>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("impostor frame {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let module = |entry: &str, stage| -> Result<vk::ShaderModule> {
            device.create_shader_module(
                &shaders.compile("impostor_draw.slang", entry, stage)?,
                entry,
            )
        };
        let vertex = module("impostor_vertex_main", ShaderStage::Vertex)?;
        let pipeline = |fragment: &str, color_formats: &[vk::Format], name: &str| {
            let module = module(fragment, ShaderStage::Fragment)?;
            let pipeline = device.create_vertex_pipeline(&VertexPipelineDesc {
                vertex: (vertex, "impostor_vertex_main"),
                fragment: (module, fragment),
                color_formats,
                depth_format: Some(vk::Format::D32_SFLOAT),
                push_constant_bytes: std::mem::size_of::<u64>() as u32,
                cull_mode: vk::CullModeFlags::NONE,
                wireframe: false,
                depth_test: true,
                alpha_blend: false,
                premultiplied: false,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let with_motion = pipeline(
            "impostor_fragment_main",
            &[HDR_FORMAT, MOTION_FORMAT],
            "impostors with motion",
        );
        let color_only = pipeline("impostor_color_main", &[HDR_FORMAT], "impostors");
        device.destroy_shader_module(vertex);
        Ok(Self {
            kinds,
            kind_table,
            instances: instance_table,
            count: instances.len() as u32,
            frames,
            with_motion: with_motion?,
            color_only: color_only?,
        })
    }

    /// How many impostors there are.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Declares the pass "impostors/draw" into `graph`: the impostors over `color` and `depth`
    /// (the resolve's colour, before the sky's compose, and the scene's depth), and their motion
    /// into `motion` when the motion vectors were made before this pass (else that pass takes
    /// it from their depth after).
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        view: &ImpostorView,
        color: ImageHandle,
        depth: ImageHandle,
        motion: Option<ImageHandle>,
    ) {
        if self.count == 0 {
            return;
        }
        let camera = view.camera.normalized();
        let frame_buffer = &self.frames[slot.index];
        frame_buffer.write(
            0,
            &[GpuImpostorFrame {
                view_proj: view.view_proj.to_cols_array(),
                previous_from_current: view.previous_from_current.to_cols_array(),
                camera_cell: camera.cell.to_array(),
                noise_frame: view.noise_frame,
                camera_local: camera.local.to_array(),
                light_scale: view.sun_illuminance * view.exposure / std::f32::consts::PI,
                sun_dir: view.sun_dir.to_array(),
                // The culls' measure of a size on screen (`ladder_crossing` in meshlet.slang).
                lod_k: view.p11 * 0.5 * view.extent.height as f32 / view.texture_lod_bias.exp2(),
                sun_color: view.sun_color.to_array(),
                znear: view.near,
                camera_in_scene: view.camera_in_scene.to_array(),
                pad: 0,
                jitter: view.jitter.to_array(),
                size: [view.extent.width as f32, view.extent.height as f32],
                instances: self.instances.address(),
                kinds: self.kind_table.address(),
                sky: view.sky.map_or(0, |s| s.address),
                tlas: view.tlas,
            }],
        );
        let address = frame_buffer.address();
        let atlases: Vec<ImageHandle> = self
            .kinds
            .iter()
            .flat_map(|k| &k.atlases)
            .map(|a| graph.import(a))
            .collect();
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        let mut pass = graph
            .pass("impostors/draw")
            .image(color, ImageAccess::ColorAttachment)
            .image(depth, ImageAccess::DepthAttachment);
        for &atlas in &atlases {
            pass = pass.image(atlas, ImageAccess::Sampled(fragment));
        }
        if let Some(motion) = motion {
            pass = pass.image(motion, ImageAccess::ColorAttachment);
        }
        if let Some(sky) = view.sky {
            pass = pass.buffer(sky.buffer, BufferAccess::ShaderRead(fragment));
        }
        if let Some(storage) = view.static_tlas.filter(|_| view.tlas != 0) {
            pass = pass.buffer(storage, BufferAccess::AccelerationStructureRead(fragment));
        }
        let (pipeline, count, extent) = match motion {
            Some(_) => (&self.with_motion, self.count, view.extent),
            None => (&self.color_only, self.count, view.extent),
        };
        pass.run(move |resources, commands| {
            let attachment = |image| {
                vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(image))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE)
            };
            let mut attachments = vec![attachment(color)];
            attachments.extend(motion.map(attachment));
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
                .color_attachments(&attachments)
                .depth_attachment(&depth_attachment);
            commands.begin_rendering(&info);
            commands.bind_pipeline(pipeline);
            commands.set_viewport_full(extent);
            commands.push_constants(pipeline, &address);
            commands.draw(6, count);
            commands.end_rendering();
            Ok(())
        });
    }
}

/// An IEEE half's value.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((h >> 10) & 0x1f);
    let mantissa = f32::from(h & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 => {
            if mantissa == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_read_back() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0), 0.0);
        assert!((half_to_f32(0x3555) - 0.333_25).abs() < 1e-4);
    }

    #[test]
    fn a_frames_axes_are_square_to_its_direction() {
        for j in 0..FRAMES {
            for i in 0..FRAMES {
                let d = frame_direction(UVec2::new(i, j));
                let (right, up) = frame_axes(d);
                assert!(right.dot(d).abs() < 1e-5 && up.dot(d).abs() < 1e-5);
                assert!(right.dot(up).abs() < 1e-5 && (up.length() - 1.0).abs() < 1e-5);
                // Up leans up: the picture is never upside down.
                assert!(up.y > 0.0, "frame ({i}, {j}): {up}");
            }
        }
    }

    /// Directions over the upper hemisphere, from a fixed sequence.
    fn hemisphere() -> impl Iterator<Item = Vec3> {
        (0..2000u32).map(|k| {
            let u = (k as f32 + 0.5) / 2000.0;
            let azimuth = k as f32 * 2.399_963;
            let y = u;
            let r = (1.0 - y * y).sqrt();
            Vec3::new(r * azimuth.cos(), y, r * azimuth.sin())
        })
    }

    #[test]
    fn a_direction_comes_back_from_its_layout_point() {
        for d in hemisphere() {
            let uv = layout_point(d);
            assert!(
                uv.cmpge(Vec2::ZERO).all() && uv.cmple(Vec2::ONE).all(),
                "{uv}"
            );
            let back = direction(uv);
            assert!(back.distance(d) < 1e-5, "{d} → {uv} → {back}");
        }
        // The middle is straight up, the edges the horizon.
        assert!(direction(Vec2::splat(0.5)).distance(Vec3::Y) < 1e-6);
        assert!(direction(Vec2::new(1.0, 0.5)).y.abs() < 1e-6);
        // Below the horizon is taken at it.
        let below = layout_point(Vec3::new(1.0, -0.3, 0.0));
        assert!(direction(below).distance(Vec3::X) < 1e-6);
    }

    #[test]
    fn every_frame_looks_from_above_the_horizon() {
        for j in 0..FRAMES {
            for i in 0..FRAMES {
                let d = frame_direction(UVec2::new(i, j));
                assert!((d.length() - 1.0).abs() < 1e-6);
                assert!(d.y > 0.1, "frame ({i}, {j}): {d}");
            }
        }
    }

    #[test]
    fn a_view_blends_the_three_frames_around_it() {
        for d in hemisphere() {
            let frames = frames_for(d);
            let total: f32 = frames.iter().map(|f| f.1).sum();
            assert!((total - 1.0).abs() < 1e-5, "{d}: {frames:?}");
            assert!(
                frames
                    .iter()
                    .all(|f| f.1 >= -1e-6 && f.0.cmplt(UVec2::splat(FRAMES)).all())
            );
            // The heaviest of the three is the frame whose middle is nearest in the layout.
            let point = layout_point(d) * FRAMES as f32 - 0.5;
            let nearest = frames
                .iter()
                .map(|f| f.0.as_vec2().distance(point))
                .fold(f32::MAX, f32::min);
            let heaviest = frames.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            assert!(
                heaviest.0.as_vec2().distance(point) <= nearest + 0.71,
                "{d}: {frames:?}"
            );
        }
        // A frame's own direction takes it whole.
        let frame = UVec2::new(3, 5);
        let frames = frames_for(frame_direction(frame));
        let own = frames.iter().find(|f| f.0 == frame).unwrap();
        assert!((own.1 - 1.0).abs() < 1e-4, "{frames:?}");
    }
}

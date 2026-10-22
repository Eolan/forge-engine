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

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_core::material::MaterialId;
use forge_geom::meshlet::MeshletMesh;
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, FrameSlot, GraphBuffer,
    GraphImage, ImageAccess, ImageDesc, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::{Mat4, UVec2, Vec2, Vec3};

use crate::CellPos;
use crate::material::GpuMaterial;
use crate::meshlet::{
    AmbientLight, CullCamera, CullFlags, DrawParams, InstanceOcclusion, MeshletRenderer,
    MeshletScene, MeshletSceneBuilder, SwRaster,
};

/// Frames a side of an impostor's layout (D-075's question 7: 8 × 8 frames of 128 px).
pub const FRAMES: u32 = 8;
/// Texels a side of a frame.
pub const FRAME_TEXELS: u32 = 128;
/// Texels a side of an atlas: every frame's.
pub const ATLAS_TEXELS: u32 = FRAMES * FRAME_TEXELS;
/// Texels a side of a frame's render per atlas texel: an edge's coverage from 16 samples.
const SAMPLES: u32 = 4;
/// Texels a side of a frame's render.
const RENDER_TEXELS: u32 = FRAME_TEXELS * SAMPLES;
/// How far the bake's camera stands from the mesh's middle, in radii of its sphere: near
/// enough to a parallel view that the frames show it without perspective.
const CAMERA_DISTANCE: f32 = 1000.0;
/// The atlases a bake makes: the base colour and its coverage, then the normals.
const ATLASES: usize = 2;

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
}

/// An impostor's atlases, read back: [`ATLAS_TEXELS`]² RGBA texels each, rows top down, frame
/// (column, row) at its [`FRAME_TEXELS`]² square.
pub struct BakedAtlases {
    /// The base colour (linear) and the share of the texel the mesh covers.
    pub albedo: Vec<[f32; 4]>,
    /// The normal in the scene frame (× 0.5 + 0.5) and the same coverage.
    pub normal: Vec<[f32; 4]>,
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
    pad: [u32; 2],
}

/// An impostor's bake on the GPU (D-075 step 4): one of its frames each app frame, with a
/// renderer of its own over a scene of the one mesh. Each frame draws the mesh from its
/// direction ([`frame_direction`]) with a camera [`CAMERA_DISTANCE`] radii off, its sphere
/// filling the picture, at full detail; the resolve writes its base colour or its normal unlit
/// ([`CullFlags::BAKE_ALBEDO`], [`CullFlags::BAKE_NORMAL`]), and the downsample takes the
/// render into the frame's square of the atlas. Then both atlases are read back.
pub struct ImpostorBake {
    scene: MeshletScene,
    renderer: MeshletRenderer,
    centre: Vec3,
    radius: f32,
    render: GraphImage,
    atlases: [GraphImage; ATLASES],
    readback: [GraphBuffer; ATLASES],
    downsample: Pipeline,
    /// The next frame to bake, the base colour's frames first.
    next: Cell<u32>,
    /// The frame slot that copied the atlases out, read once it comes round again.
    copied: Cell<Option<usize>>,
    baked: RefCell<Option<BakedAtlases>>,
    /// Whether the atlases were read back.
    read: Cell<bool>,
}

impl ImpostorBake {
    /// Prepares the bake of `source`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        source: &BakeSource,
    ) -> Result<Self> {
        let mut builder = MeshletSceneBuilder::new();
        let mesh = builder.add_mesh(&source.mesh);
        builder.set_material_rows(source.rows.clone());
        builder.add_instance_with_material(mesh, Mat4::IDENTITY, source.material);
        let scene = builder.build(device)?;
        let extent = vk::Extent2D {
            width: RENDER_TEXELS,
            height: RENDER_TEXELS,
        };
        let renderer = MeshletRenderer::for_bake(device, shaders, extent)?;
        let image = |name: &'static str, side: u32| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: side,
                    height: side,
                    format: vk::Format::R16G16B16A16_SFLOAT,
                    usage: vk::ImageUsageFlags::STORAGE
                        | vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::TRANSFER_SRC,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let readback = |name: &'static str| {
            device
                .create_buffer(BufferDesc {
                    size: u64::from(ATLAS_TEXELS) * u64::from(ATLAS_TEXELS) * 8,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name,
                })
                .map(GraphBuffer::new)
        };
        let entry = "impostor_downsample_main";
        let module = device.create_shader_module(
            &shaders.compile("impostor.slang", entry, ShaderStage::Compute)?,
            entry,
        )?;
        let downsample = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, entry),
            push_constant_bytes: std::mem::size_of::<DownsamplePush>() as u32,
            name: "impostor downsample",
        });
        device.destroy_shader_module(module);
        Ok(Self {
            scene,
            renderer,
            centre: Vec3::from_array(source.mesh.center),
            radius: source.mesh.radius.max(1e-3),
            render: image("impostor render", RENDER_TEXELS)?,
            atlases: [
                image("impostor albedo", ATLAS_TEXELS)?,
                image("impostor normal", ATLAS_TEXELS)?,
            ],
            readback: [
                readback("impostor albedo read back")?,
                readback("impostor normal read back")?,
            ],
            downsample: downsample?,
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

    /// Declares this frame's part of the bake into `graph`: the next frame drawn into its
    /// square, or the atlases copied out after the last, or nothing while the copy is in
    /// flight. Returns whether the bake still has work (false once the atlases are read).
    pub fn record<'f>(&'f self, graph: &mut FrameGraph<'f>, slot: FrameSlot) -> Result<bool> {
        let total = ATLASES as u32 * FRAMES * FRAMES;
        if let Some(copied) = self.copied.get() {
            // The slot comes round once its frame is done: the copies have landed.
            if copied == slot.index && !self.read.get() {
                self.read.set(true);
                let read = |buffer: &GraphBuffer| {
                    let mut halves = vec![0u16; (ATLAS_TEXELS * ATLAS_TEXELS * 4) as usize];
                    buffer.read(0, &mut halves);
                    halves
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|t| t.map(half_to_f32))
                        .collect::<Vec<_>>()
                };
                *self.baked.borrow_mut() = Some(BakedAtlases {
                    albedo: read(&self.readback[0]),
                    normal: read(&self.readback[1]),
                });
            }
            return Ok(!self.read.get());
        }
        if self.next.get() == total {
            self.copied.set(Some(slot.index));
            let this = self;
            let extent = vk::Extent2D {
                width: ATLAS_TEXELS,
                height: ATLAS_TEXELS,
            };
            for (atlas, readback) in this.atlases.iter().zip(&this.readback) {
                let image = graph.import(atlas);
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
            return Ok(true);
        }
        let k = self.next.get();
        self.next.set(k + 1);
        let this = self;
        let atlas_index = (k / (FRAMES * FRAMES)) as usize;
        let frame = UVec2::new(k % FRAMES, (k / FRAMES) % FRAMES);
        let direction = frame_direction(frame);
        let (right, up) = frame_axes(direction);
        let distance = CAMERA_DISTANCE * this.radius;
        let eye = this.centre + direction * distance;
        // Camera-relative → view: the rotation alone (D-004), looking along -direction.
        let view = Mat4::from_cols(
            right.extend(0.0),
            up.extend(0.0),
            direction.extend(0.0),
            glam::Vec4::W,
        )
        .transpose();
        let near = distance - 2.0 * this.radius;
        let fov = 2.0 * (this.radius / distance).atan();
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
        let targets = this.renderer.draw(
            graph,
            slot,
            DrawParams {
                scene: &this.scene,
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
        let render = graph.import(&this.render);
        this.renderer.resolve(
            graph,
            slot,
            targets,
            render,
            extent,
            Some([0.0; 4]),
            AmbientLight::default(),
        );
        let atlas = graph.import(&this.atlases[atlas_index]);
        let downsample = &this.downsample;
        let tile = frame * FRAME_TEXELS;
        graph
            .pass("impostor/downsample")
            .image(
                render,
                ImageAccess::StorageRead(vk::PipelineStageFlags2::COMPUTE_SHADER),
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
                        pad: [0; 2],
                    },
                );
                commands.dispatch(FRAME_TEXELS / 8, FRAME_TEXELS / 8, 1);
                Ok(())
            });
        Ok(true)
    }

    /// Writes the atlases as PNG images to `dir` (`{name}-albedo.png` in sRGB with its
    /// coverage as alpha, `{name}-normal.png`), for review.
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
        let save = |texels: &[[f32; 4]], path: std::path::PathBuf, colour: &dyn Fn(f32) -> u8| {
            let bytes: Vec<u8> = texels
                .iter()
                .flat_map(|t| [colour(t[0]), colour(t[1]), colour(t[2]), unorm(t[3])])
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
        save(
            &atlases.albedo,
            dir.join(format!("{name}-albedo.png")),
            &srgb,
        )?;
        save(
            &atlases.normal,
            dir.join(format!("{name}-normal.png")),
            &unorm,
        )
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

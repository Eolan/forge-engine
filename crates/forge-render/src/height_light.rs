//! The light the city leaves a surface (D-108, step 6, `shaders/height_light.slang`): with no
//! probes, the open sky's light on a surface, gathered against the heights seen from above
//! ([`crate::shadow_rings::RingCache::heights`], drawn by [`crate::MeshletRenderer`]).
//!
//! Each frame, at half resolution, before the resolve: the view depth; a few directions a pixel
//! marched against the heights; one spatial pass of a fixed radius; one temporal pass. The resolve
//! reads the half-resolution texel nearest in depth to its pixel: the share of the open sky's
//! irradiance the surface takes, and the share of the sky its mirror direction sees.
//!
//! A reflection cube's face takes the same light when it is drawn ([`HeightLight::draw_face`]):
//! more directions a texel and no history, since a face is drawn once.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, FrameSlot,
    GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::{Mat4, Vec3};

use crate::SkyLight;
use crate::meshlet::SunRings;
use crate::shadow_rings::HEIGHT_LEVELS;

/// The faces a frame may gather for besides the camera's view: the cubes' budget at most.
pub const FACE_VIEWS: usize = 6;
/// Directions a texel of a cube's face marches: it is drawn once, with no history to average them.
const FACE_DIRECTIONS: u32 = 32;
/// Half-resolution pixels between two the camera's view marches in a frame, each way: one pixel
/// in four, which goes round with the frames. The filter spreads them and the history holds the
/// rest; every pixel marched each frame cost four times the pass (3.0 ms against 0.8 on an RTX
/// 5060 Laptop GPU at 1600 x 900).
const CAMERA_SPACING: u32 = 2;
/// A view's block in a frame slot's buffer, bytes.
const BLOCK_STRIDE: u64 = 256;

/// Mirrors `Gather` in `height_light.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuGather {
    world_from_clip: [f32; 16],
    previous_from_current: [f32; 16],
    clip_w: [f32; 4],
    offset: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    heights: u64,
    rings: u64,
    sky: u64,
    directions: u32,
    steps: u32,
    frame: u32,
    reset: u32,
    spacing: u32,
    first: [u32; 2],
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuGather>() as u64 <= BLOCK_STRIDE);

/// Mirrors `Push` in `height_light.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    gather: u64,
    depth_image: u32,
    z_image: u32,
    last_z_image: u32,
    input_image: u32,
    last_image: u32,
    output_image: u32,
    width: u32,
    height: u32,
    last_count_image: u32,
    count_image: u32,
}

/// The heights seen from above as a pass reads them (`heights.slang`): the levels' `HeightSet`
/// record and their images, which the pass declares sampled.
#[derive(Clone, Copy, Debug)]
pub struct CityHeights {
    /// The `HeightSet` record's device address.
    pub set: u64,
    /// The levels' images this frame drew, finest first.
    pub layers: [ImageHandle; HEIGHT_LEVELS],
}

/// What a frame's pass needs of the camera's view.
#[derive(Clone, Copy, Debug)]
pub struct HeightLightView {
    /// The view-projection the depth was drawn with, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// From this frame's clip space to the last's.
    pub previous_from_current: Mat4,
    /// A counter for the noise.
    pub frame: u32,
    /// There is no history to read.
    pub reset: bool,
}

/// What a view's passes read besides its depth.
#[derive(Clone, Copy, Debug)]
pub struct HeightLightScene {
    /// The heights.
    pub heights: CityHeights,
    /// The sun's rings, for a blocked direction's sun; `None`: unshadowed.
    pub rings: Option<SunRings>,
    /// The sky's light.
    pub sky: SkyLight,
    /// Towards the key light.
    pub sun_dir: Vec3,
    /// Its colour, as the resolve's.
    pub sun_color: Vec3,
    /// From the view's camera to the one the heights and the rings were drawn for.
    pub offset: Vec3,
}

/// The settings (`r.gi.heights_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightLightSettings {
    /// How far a direction is marched, metres.
    pub reach: f32,
    /// What a blocked direction's face is taken to reflect: a guess, a city's albedo by default
    /// (the sky's own ground is 0.3, which measured a quarter of a stop bright in the city).
    pub reflectance: f32,
    /// Directions a marched pixel takes each frame (the camera's view marches one pixel in four).
    pub directions: u32,
    /// Steps a direction takes.
    pub steps: u32,
    /// The street cubes' ambient light over the gathered light where they stand, else left out
    /// (a room's cube stays its room's light either way).
    pub street_cubes: bool,
}

impl Default for HeightLightSettings {
    fn default() -> Self {
        Self {
            reach: 256.0,
            reflectance: 0.15,
            directions: 4,
            steps: 12,
            street_cubes: false,
        }
    }
}

/// What a view's passes give the resolve: the gathered light and the view depth it is picked by,
/// both at half the view's resolution.
#[derive(Clone, Copy, Debug)]
pub struct HeightLightImages {
    /// rgb: the share of the open sky's irradiance the surface takes; a: the share of the sky its
    /// mirror direction sees.
    pub light: ImageHandle,
    /// The view depth of each texel.
    pub z: ImageHandle,
}

/// The passes and the images that outlive a frame.
pub struct HeightLight {
    z: Pipeline,
    trace: Pipeline,
    filter: Pipeline,
    temporal: Pipeline,
    /// The camera's light of the frames before, written in turn, and their view depths.
    history: [GraphImage; 2],
    depths: [GraphImage; 2],
    /// How many frames each pixel of `history` is the average of.
    counts: [GraphImage; 2],
    /// A cube's face's light and view depth, at half the face, and its light before the filter:
    /// no transient, so that a frame's are laid out the same however many faces it draws.
    face_light: GraphImage,
    face_z: GraphImage,
    face_raw: GraphImage,
    /// Per frame slot, the views' blocks: the camera's, then the faces'.
    blocks: Vec<Buffer>,
    /// Which of `history` the last frame wrote.
    turn: Cell<usize>,
    /// The renderer's frame the camera's passes last ran in: any other than the one before this
    /// one, and its images hold nothing of the frame before.
    last_frame: Cell<Option<u64>>,
    extent: vk::Extent2D,
    face: u32,
}

fn half(extent: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D {
        width: (extent.width / 2).max(1),
        height: (extent.height / 2).max(1),
    }
}

impl HeightLight {
    /// Makes the passes and images for a frame of `extent` and cube faces `face` texels a side.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
        face: u32,
    ) -> Result<Self> {
        let make = |entry: &'static str, name: &'static str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("height_light.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<Push>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let image = |size: vk::Extent2D, format: vk::Format, name: &'static str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: size.width,
                    height: size.height,
                    format,
                    usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let size = half(extent);
        let face_size = half(vk::Extent2D {
            width: face,
            height: face,
        });
        let (light, depth) = (vk::Format::R16G16B16A16_SFLOAT, vk::Format::R32_SFLOAT);
        let blocks = (0..forge_gpu::FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (1 + FACE_VIEWS as u64) * BLOCK_STRIDE,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("height light views {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            z: make("z_main", "height light depth")?,
            trace: make("trace_main", "height light")?,
            filter: make("filter_main", "height light filter")?,
            temporal: make("temporal_main", "height light history")?,
            history: [
                image(size, light, "height light history 0")?,
                image(size, light, "height light history 1")?,
            ],
            depths: [
                image(size, depth, "height light depth 0")?,
                image(size, depth, "height light depth 1")?,
            ],
            counts: [
                image(size, vk::Format::R16G16_SFLOAT, "height light frames 0")?,
                image(size, vk::Format::R16G16_SFLOAT, "height light frames 1")?,
            ],
            face_light: image(face_size, light, "height light of a face")?,
            face_z: image(face_size, depth, "height light depth of a face")?,
            face_raw: image(face_size, light, "height light of a face, raw")?,
            blocks,
            turn: Cell::new(0),
            last_frame: Cell::new(None),
            extent,
            face,
        })
    }

    /// The frame size it was made for.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// The sampled indices of the light and the view depth the camera's passes will write this
    /// frame, before [`Self::draw`], which the frame's block names for the resolve. It changes
    /// nothing: a frame's block is built more than once.
    pub fn next_indices(&self) -> [u32; 2] {
        let next = 1 - self.turn.get();
        [
            self.history[next].sampled().0,
            self.depths[next].sampled().0,
        ]
    }

    /// What its images hold is not of the frame before (the rings were off for a while): the next
    /// frame reads no history.
    pub fn invalidate(&self) {
        self.last_frame.set(None);
    }

    /// The sampled indices of a face's light and view depth, for its block.
    pub fn face_indices(&self) -> [u32; 2] {
        [self.face_light.sampled().0, self.face_z.sampled().0]
    }

    #[allow(clippy::too_many_arguments)]
    fn block(
        &self,
        slot: FrameSlot,
        index: u64,
        view_proj: Mat4,
        previous_from_current: Mat4,
        scene: &HeightLightScene,
        settings: HeightLightSettings,
        directions: u32,
        frame: u32,
        reset: bool,
        spacing: u32,
    ) -> u64 {
        let reach = settings.reach.max(1.0);
        let block = GpuGather {
            world_from_clip: view_proj.inverse().to_cols_array(),
            previous_from_current: previous_from_current.to_cols_array(),
            clip_w: view_proj.row(3).to_array(),
            offset: scene.offset.extend(0.0).to_array(),
            sun_dir: scene.sun_dir.normalize_or_zero().extend(reach).to_array(),
            sun_color: scene
                .sun_color
                .extend(settings.reflectance.clamp(0.0, 1.0))
                .to_array(),
            heights: scene.heights.set,
            rings: SunRings::address(scene.rings),
            sky: scene.sky.address,
            directions: directions.clamp(1, 64),
            steps: settings.steps.clamp(2, 64),
            frame,
            reset: u32::from(reset),
            spacing,
            // One in four goes round in four frames; all of them start at the first.
            first: if spacing > 1 {
                [frame & 1, (frame >> 1) & 1]
            } else {
                [0; 2]
            },
            pad: 0,
        };
        let buffer = &self.blocks[slot.index];
        buffer.write(index * BLOCK_STRIDE, &[block]);
        buffer.address() + index * BLOCK_STRIDE
    }

    /// The view depth, then the march: `raw` holds the gathered light of `depth`'s view.
    #[allow(clippy::too_many_arguments)]
    fn gather<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        depth: ImageHandle,
        z: ImageHandle,
        raw: ImageHandle,
        scene: &HeightLightScene,
        block: u64,
        size: vk::Extent2D,
        spacing: u32,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let groups = (size.width.div_ceil(8), size.height.div_ceil(8));
        let marched = (
            size.width.div_ceil(spacing).div_ceil(8),
            size.height.div_ceil(spacing).div_ceil(8),
        );
        let base = Push {
            gather: block,
            depth_image: 0,
            z_image: 0,
            last_z_image: 0,
            input_image: 0,
            last_image: 0,
            output_image: 0,
            width: size.width,
            height: size.height,
            last_count_image: 0,
            count_image: 0,
        };
        let (z_pass, trace) = (&self.z, &self.trace);
        graph
            .pass("gi/height depth")
            .image(depth, ImageAccess::Sampled(compute))
            .image(z, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(z_pass);
                commands.push_constants(
                    z_pass,
                    &Push {
                        depth_image: resources.sampled(depth).0,
                        output_image: resources.storage(z, 0).0,
                        ..base
                    },
                );
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        let mut builder = graph
            .pass("gi/height trace")
            .image(z, ImageAccess::Sampled(compute))
            .image(raw, ImageAccess::StorageWrite(compute))
            .buffer(scene.sky.buffer, BufferAccess::ShaderRead(compute))
            .image(scene.sky.table, ImageAccess::Sampled(compute));
        for layer in scene.heights.layers {
            builder = builder.image(layer, ImageAccess::Sampled(compute));
        }
        SunRings::declare(scene.rings, builder, compute).run(move |resources, commands| {
            commands.bind_pipeline(trace);
            commands.push_constants(
                trace,
                &Push {
                    z_image: resources.sampled(z).0,
                    output_image: resources.storage(raw, 0).0,
                    ..base
                },
            );
            commands.dispatch(marched.0, marched.1, 1);
            Ok(())
        });
    }

    /// The spatial pass: `raw` into `filtered`, by `z`.
    fn filter<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        z: ImageHandle,
        raw: ImageHandle,
        filtered: ImageHandle,
        block: u64,
        size: vk::Extent2D,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let filter = &self.filter;
        graph
            .pass("gi/height filter")
            .image(raw, ImageAccess::Sampled(compute))
            .image(z, ImageAccess::Sampled(compute))
            .image(filtered, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(filter);
                commands.push_constants(
                    filter,
                    &Push {
                        gather: block,
                        depth_image: 0,
                        z_image: resources.sampled(z).0,
                        last_z_image: 0,
                        input_image: resources.sampled(raw).0,
                        last_image: 0,
                        output_image: resources.storage(filtered, 0).0,
                        width: size.width,
                        height: size.height,
                        last_count_image: 0,
                        count_image: 0,
                    },
                );
                commands.dispatch(size.width.div_ceil(8), size.height.div_ceil(8), 1);
                Ok(())
            });
    }

    fn transient<'f>(
        graph: &mut FrameGraph<'f>,
        name: &'static str,
        size: vk::Extent2D,
    ) -> ImageHandle {
        graph.transient(TransientDesc {
            name,
            width: size.width,
            height: size.height,
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        })
    }

    /// Declares the camera's passes over `depth` in the renderer's frame `frame` (a count that
    /// goes up by one a frame), and returns what the resolve reads: the images
    /// [`Self::next_indices`] named before it.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        depth: ImageHandle,
        view: HeightLightView,
        scene: HeightLightScene,
        settings: HeightLightSettings,
    ) -> HeightLightImages {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let size = half(self.extent);
        let turn = 1 - self.turn.get();
        self.turn.set(turn);
        let before = self.last_frame.replace(Some(frame));
        let reset = view.reset || before.is_none() || before != frame.checked_sub(1);
        let block = self.block(
            slot,
            0,
            view.view_proj,
            view.previous_from_current,
            &scene,
            settings,
            settings.directions,
            view.frame,
            reset,
            CAMERA_SPACING,
        );
        let z = graph.import(&self.depths[turn]);
        let last_z = graph.import(&self.depths[1 - turn]);
        let next = graph.import(&self.history[turn]);
        let last = graph.import(&self.history[1 - turn]);
        let count = graph.import(&self.counts[turn]);
        let last_count = graph.import(&self.counts[1 - turn]);
        let raw = Self::transient(
            graph,
            "height light raw",
            vk::Extent2D {
                width: size.width.div_ceil(CAMERA_SPACING),
                height: size.height.div_ceil(CAMERA_SPACING),
            },
        );
        let filtered = Self::transient(graph, "height light filtered", size);
        self.gather(graph, depth, z, raw, &scene, block, size, CAMERA_SPACING);
        self.filter(graph, z, raw, filtered, block, size);
        let temporal = &self.temporal;
        graph
            .pass("gi/height history")
            .image(filtered, ImageAccess::Sampled(compute))
            .image(z, ImageAccess::Sampled(compute))
            .image(last, ImageAccess::Sampled(compute))
            .image(last_z, ImageAccess::Sampled(compute))
            .image(last_count, ImageAccess::Sampled(compute))
            .image(next, ImageAccess::StorageWrite(compute))
            .image(count, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(temporal);
                commands.push_constants(
                    temporal,
                    &Push {
                        gather: block,
                        depth_image: 0,
                        z_image: resources.sampled(z).0,
                        last_z_image: resources.sampled(last_z).0,
                        input_image: resources.sampled(filtered).0,
                        last_image: resources.sampled(last).0,
                        output_image: resources.storage(next, 0).0,
                        width: size.width,
                        height: size.height,
                        last_count_image: resources.sampled(last_count).0,
                        count_image: resources.storage(count, 0).0,
                    },
                );
                commands.dispatch(size.width.div_ceil(8), size.height.div_ceil(8), 1);
                Ok(())
            });
        HeightLightImages { light: next, z }
    }

    /// Declares the passes of a cube's face over its `depth`, the `index`-th of the frame's
    /// (under [`FACE_VIEWS`]): its light marched for its own texels, with no history.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_face<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        index: usize,
        depth: ImageHandle,
        view_proj: Mat4,
        scene: HeightLightScene,
        settings: HeightLightSettings,
    ) -> HeightLightImages {
        debug_assert!(index < FACE_VIEWS);
        let size = half(vk::Extent2D {
            width: self.face,
            height: self.face,
        });
        let block = self.block(
            slot,
            1 + index as u64,
            view_proj,
            Mat4::IDENTITY,
            &scene,
            settings,
            FACE_DIRECTIONS,
            index as u32,
            true,
            1,
        );
        let z = graph.import(&self.face_z);
        let light = graph.import(&self.face_light);
        let raw = graph.import(&self.face_raw);
        self.gather(graph, depth, z, raw, &scene, block, size, 1);
        self.filter(graph, z, raw, light, block, size);
        HeightLightImages { light, z }
    }
}

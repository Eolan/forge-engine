//! The last few metres of the diffuse light, from the screen (D-108, step 4, `shaders/screen_gi.slang`):
//! a visibility bitmask at half resolution over whatever gives the base term.
//!
//! Each frame: the view depth at half resolution; the bitmask, reading the last frame's light; one
//! spatial pass of a fixed radius; one temporal pass. The resolve reads the half-resolution texel
//! nearest in depth to its pixel. After the resolve the frame's colour is kept, at half resolution,
//! for the next frame.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, GraphImage, ImageAccess, ImageDesc, ImageHandle,
    Pipeline, Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};
use glam::Mat4;

/// Mirrors `Push` in `screen_gi.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    previous_from_current: [f32; 16],
    ndc_to_view_mul: [f32; 2],
    ndc_to_view_add: [f32; 2],
    depth_image: u32,
    z_image: u32,
    light_image: u32,
    input_image: u32,
    output_image: u32,
    width: u32,
    height: u32,
    frame: u32,
    near: f32,
    radius: f32,
    thickness: f32,
    pixels: f32,
    scale: f32,
    slices: u32,
    steps: u32,
    reset: u32,
}

/// What a frame's pass needs of its view.
#[derive(Clone, Copy, Debug)]
pub struct ScreenGiView {
    /// The projection the depth was drawn with (jitter included), reversed-Z, infinite.
    pub projection: Mat4,
    /// From this frame's clip space to the last's.
    pub previous_from_current: Mat4,
    /// A counter for the noise.
    pub frame: u32,
    /// There is no history to read.
    pub reset: bool,
}

/// What a frame's passes give: the near light, and the last frame's light as this frame imported it,
/// which [`ScreenGi::keep_light`] writes for the next.
#[derive(Clone, Copy, Debug)]
pub struct ScreenGiImages {
    /// The near light at half resolution: rgb, and in alpha the view depth the resolve picks by.
    pub near: ImageHandle,
    /// The kept light.
    pub light: ImageHandle,
}

/// The settings (`r.gi.screen_*`).
#[derive(Clone, Copy, Debug)]
pub struct ScreenGiSettings {
    /// How far a surface lights another, metres: the light fades out to here.
    pub radius: f32,
    /// How deep a surface is taken to be behind what the depth shows, metres.
    pub thickness: f32,
    /// Slices per pixel.
    pub slices: u32,
    /// Steps per side of a slice.
    pub steps: u32,
}

impl Default for ScreenGiSettings {
    fn default() -> Self {
        Self {
            radius: 2.0,
            thickness: 0.4,
            slices: 2,
            steps: 6,
        }
    }
}

/// The passes and the images that outlive a frame.
pub struct ScreenGi {
    z: Pipeline,
    trace: Pipeline,
    filter: Pipeline,
    temporal: Pipeline,
    keep: Pipeline,
    /// The last frame's light, at half resolution.
    light: GraphImage,
    /// The near light of the frames before, written in turn.
    history: [GraphImage; 2],
    /// Which of `history` the last frame wrote.
    turn: usize,
    /// Its images hold nothing of the frames before (new, or the pass was off): the next frame
    /// reads no history and no last light.
    stale: bool,
    /// This frame's passes read no history (`stale` when it turned).
    reset_now: bool,
    extent: vk::Extent2D,
}

fn half(extent: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D {
        width: (extent.width / 2).max(1),
        height: (extent.height / 2).max(1),
    }
}

impl ScreenGi {
    /// Makes the passes and images for a frame of `extent`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let make = |entry: &'static str, name: &'static str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("screen_gi.slang", entry, ShaderStage::Compute)?,
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
        let size = half(extent);
        let image = |name: &'static str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: size.width,
                    height: size.height,
                    format: vk::Format::R16G16B16A16_SFLOAT,
                    usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::STORAGE,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        Ok(Self {
            z: make("z_main", "screen gi depth")?,
            trace: make("trace_main", "screen gi")?,
            filter: make("filter_main", "screen gi filter")?,
            temporal: make("temporal_main", "screen gi history")?,
            keep: make("light_main", "screen gi light")?,
            light: image("screen gi light")?,
            history: [image("screen gi history 0")?, image("screen gi history 1")?],
            turn: 0,
            stale: true,
            reset_now: true,
            extent,
        })
    }

    /// The frame size it was made for.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// Turns to the other history image, once a frame, before [`Self::draw`].
    pub fn advance(&mut self) {
        self.turn = 1 - self.turn;
        self.reset_now = std::mem::take(&mut self.stale);
    }

    /// What its images hold is not of the frame before (the pass was off for a while): the next
    /// frame starts afresh.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// Declares the frame's passes over `depth`, and returns the near light and the kept light's
    /// handle for [`Self::keep_light`].
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        depth: ImageHandle,
        view: ScreenGiView,
        settings: ScreenGiSettings,
    ) -> ScreenGiImages {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let size = half(self.extent);
        let (width, height) = (size.width, size.height);
        let p = view.projection;
        let (p00, p11) = (p.x_axis.x, p.y_axis.y);
        // Reversed-Z, infinite: as `gtao.rs` reads its projection.
        let (jx, jy) = (-p.z_axis.x, -p.z_axis.y);
        let near = p.w_axis.z;
        let base = Push {
            previous_from_current: view.previous_from_current.to_cols_array(),
            ndc_to_view_mul: [2.0 / p00, -2.0 / p11],
            ndc_to_view_add: [(-1.0 - jx) / p00, (1.0 - jy) / p11],
            depth_image: 0,
            z_image: 0,
            light_image: 0,
            input_image: 0,
            output_image: 0,
            width,
            height,
            frame: view.frame,
            near,
            radius: settings.radius,
            thickness: settings.thickness,
            pixels: p11 * 0.5 * height as f32,
            scale: 1.0,
            slices: settings.slices,
            steps: settings.steps,
            reset: u32::from(view.reset || self.reset_now),
        };
        let image = |name: &'static str, format: vk::Format| TransientDesc {
            name,
            width,
            height,
            format,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let z = graph.transient(image("screen gi depth", vk::Format::R32_SFLOAT));
        let raw = graph.transient(image("screen gi raw", vk::Format::R16G16B16A16_SFLOAT));
        let filtered =
            graph.transient(image("screen gi filtered", vk::Format::R16G16B16A16_SFLOAT));
        let light = graph.import(&self.light);
        let (last, next) = (
            graph.import(&self.history[1 - self.turn]),
            graph.import(&self.history[self.turn]),
        );
        let groups = (width.div_ceil(8), height.div_ceil(8));
        let (z_pass, trace, filter, temporal) =
            (&self.z, &self.trace, &self.filter, &self.temporal);
        graph
            .pass("gi/screen depth")
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
        graph
            .pass("gi/screen trace")
            .image(z, ImageAccess::Sampled(compute))
            .image(light, ImageAccess::Sampled(compute))
            .image(raw, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(trace);
                commands.push_constants(
                    trace,
                    &Push {
                        z_image: resources.sampled(z).0,
                        light_image: resources.sampled(light).0,
                        output_image: resources.storage(raw, 0).0,
                        ..base
                    },
                );
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        graph
            .pass("gi/screen filter")
            .image(raw, ImageAccess::Sampled(compute))
            .image(filtered, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(filter);
                commands.push_constants(
                    filter,
                    &Push {
                        input_image: resources.sampled(raw).0,
                        output_image: resources.storage(filtered, 0).0,
                        ..base
                    },
                );
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        graph
            .pass("gi/screen history")
            .image(filtered, ImageAccess::Sampled(compute))
            .image(last, ImageAccess::Sampled(compute))
            .image(next, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(temporal);
                commands.push_constants(
                    temporal,
                    &Push {
                        input_image: resources.sampled(filtered).0,
                        light_image: resources.sampled(last).0,
                        output_image: resources.storage(next, 0).0,
                        ..base
                    },
                );
                commands.dispatch(groups.0, groups.1, 1);
                Ok(())
            });
        ScreenGiImages { near: next, light }
    }

    /// Keeps the frame's colour at half resolution for the next frame's trace: after the resolve,
    /// into `light` as [`Self::draw`] gave it (one handle, so the graph orders this write after the
    /// trace's read). `scale` is what the frame's colour was multiplied by (the sun's illuminance,
    /// the exposure and 1/π).
    pub fn keep_light<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        light: ImageHandle,
        color: ImageHandle,
        depth: ImageHandle,
        scale: f32,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let size = half(self.extent);
        let pipeline = &self.keep;
        // It reads back the colour the resolve's passes wrote: behind a full barrier (#349).
        graph
            .pass("gi/screen keep")
            .full_barrier()
            .image(color, ImageAccess::Sampled(compute))
            .image(depth, ImageAccess::Sampled(compute))
            .image(light, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &Push {
                        previous_from_current: Mat4::IDENTITY.to_cols_array(),
                        ndc_to_view_mul: [0.0; 2],
                        ndc_to_view_add: [0.0; 2],
                        depth_image: resources.sampled(depth).0,
                        z_image: 0,
                        light_image: 0,
                        input_image: resources.sampled(color).0,
                        output_image: resources.storage(light, 0).0,
                        width: size.width,
                        height: size.height,
                        frame: 0,
                        near: 0.0,
                        radius: 0.0,
                        thickness: 0.0,
                        pixels: 0.0,
                        scale,
                        slices: 0,
                        steps: 0,
                        reset: 0,
                    },
                );
                commands.dispatch(size.width.div_ceil(8), size.height.div_ceil(8), 1);
                Ok(())
            });
    }
}

//! A procedural starfield background (`shaders/starfield.slang`): one full-screen triangle,
//! stars and a faint nebula hashed from the view direction. No depth is written.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::Mat4;

/// Mirrors `Push` in `starfield.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    inv_view_proj: [f32; 16],
    sun_dir: [f32; 3],
    exposure: f32,
    planet_dir: [f32; 3],
    planet_angle: f32,
}

/// The starfield pass.
pub struct Starfield {
    pipeline: Pipeline,
    /// Brightness multiplier.
    pub exposure: f32,
    /// Direction to a distant planet drawn in the sky.
    pub planet_dir: glam::Vec3,
    /// Angular radius of the planet in radians; 0 hides it.
    pub planet_angle: f32,
}

impl Starfield {
    /// Compiles the pipeline for `color_format`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        color_format: vk::Format,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("starfield.slang", "vert_main", ShaderStage::Vertex)?,
            "starfield vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("starfield.slang", "frag_main", ShaderStage::Fragment)?,
            "starfield fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[color_format],
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            alpha_blend: false,
            name: "starfield",
        })?;
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        Ok(Self {
            pipeline,
            exposure: 1.0,
            planet_dir: glam::Vec3::new(-0.35, 0.12, -1.0).normalize(),
            planet_angle: 0.0,
        })
    }

    /// Declares the pass that draws the background over the whole of `color`, clearing
    /// nothing: the pass covers every pixel.
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        color: ImageHandle,
        extent: vk::Extent2D,
        view_proj: Mat4,
        sun_dir: glam::Vec3,
    ) {
        let push = Push {
            inv_view_proj: view_proj.inverse().to_cols_array(),
            sun_dir: sun_dir.to_array(),
            exposure: self.exposure,
            planet_dir: self.planet_dir.to_array(),
            planet_angle: self.planet_angle,
        };
        let pipeline = &self.pipeline;
        graph
            .pass("sky/starfield + planet")
            .image(color, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let attachments = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(color))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments);
                commands.begin_rendering(&info);
                commands.bind_pipeline(pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(pipeline, &push);
                commands.draw(3, 1);
                commands.end_rendering();
                Ok(())
            });
    }
}

//! Supersampling for screenshots (D-045, `AppConfig::ssaa`): the demo draws its frame at twice
//! the window's width and height ([`crate::Context::extent`] says so), and this pass takes each
//! window pixel as the mean of its four (`shaders/ssaa.slang`). It costs about four times the
//! frame, so it suits captures and stills, not play.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};

/// The frame's size over the window's, each way.
pub(crate) const FACTOR: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SsaaPush {
    image: u32,
}

/// The pass that filters the large frame down to the window.
pub(crate) struct Ssaa {
    pipeline: Pipeline,
    format: vk::Format,
}

impl Ssaa {
    /// Compiles the pass for a window image of `format`.
    pub(crate) fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("ssaa.slang", "vert_main", ShaderStage::Vertex)?,
            "ssaa vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("ssaa.slang", "frag_main", ShaderStage::Fragment)?,
            "ssaa fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[format],
            push_constant_bytes: std::mem::size_of::<SsaaPush>() as u32,
            alpha_blend: false,
            depth_test: None,
            depth_write: false,
            name: "ssaa",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        Ok(Self {
            pipeline: pipeline?,
            format,
        })
    }

    /// The window image's format the pass was compiled for.
    pub(crate) fn format(&self) -> vk::Format {
        self.format
    }

    /// Declares "post/ssaa": `large` (FACTOR times `extent` each way) into `target` (`extent`).
    pub(crate) fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        large: ImageHandle,
        target: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let pipeline = &self.pipeline;
        graph
            .pass("post/ssaa")
            .image(
                large,
                ImageAccess::Sampled(vk::PipelineStageFlags2::FRAGMENT_SHADER),
            )
            .image(target, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let attachments = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(target))
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
                commands.push_constants(
                    pipeline,
                    &SsaaPush {
                        image: resources.sampled(large).0,
                    },
                );
                commands.draw(3, 1);
                commands.end_rendering();
                Ok(())
            });
    }
}

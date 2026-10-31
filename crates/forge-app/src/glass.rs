//! The frosted glass behind the game's menu (#377, the owner on 2026-10-31: "a glass gaussian
//! transparency effect but almost opaque still to not get in the way of visibility", "or full
//! screen"): the frame as the demo drew it, copied, brought down to a quarter of its size, blurred
//! by a Gaussian across and down, and drawn back over the whole window, mostly a dark tint
//! (`shaders/menu_glass.slang`). Five small passes (`app/menu glass ...`), only while the menu
//! shows; made the first time it opens.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, TransientDesc, vk,
};

/// Must match `GlassPush` in `menu_glass.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlassPush {
    image: u32,
    encoding: u32,
    ui_white: f32,
    axis: u32,
    texel: [f32; 2],
    pad: [f32; 2],
}

/// The glass's pipelines, for the window's format.
pub(crate) struct Glass {
    device: Arc<Device>,
    down: Pipeline,
    blur: Pipeline,
    glass: Pipeline,
    format: vk::Format,
    encoding: u32,
    ui_white: f32,
}

/// The quarter-size images' format: one every GPU filters and draws to, whatever the window's
/// (HDR10's A2R10G10B10 need not filter); the frame's values as read (linear light, or PQ codes).
const QUARTER_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// The size a quarter of `extent`'s, at least a pixel.
fn quarter(extent: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D {
        width: extent.width.div_ceil(4).max(1),
        height: extent.height.div_ceil(4).max(1),
    }
}

impl Glass {
    /// Compiles the passes for a window image of `format`.
    pub(crate) fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<Self> {
        let [down, blur, glass] = Self::pipelines(device, shaders, format)?;
        Ok(Self {
            device: Arc::clone(device),
            down,
            blur,
            glass,
            format,
            encoding: crate::overlay::encoding(format),
            ui_white: 203.0,
        })
    }

    fn pipelines(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<[Pipeline; 3]> {
        let vertex = device.create_shader_module(
            &shaders.compile("menu_glass.slang", "vert_main", ShaderStage::Vertex)?,
            "menu glass vs",
        )?;
        let make =
            |entry: &'static str, name: &'static str, format: vk::Format| -> Result<Pipeline> {
                let fragment = device.create_shader_module(
                    &shaders.compile("menu_glass.slang", entry, ShaderStage::Fragment)?,
                    name,
                )?;
                let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
                    vertex: (vertex, "vert_main"),
                    fragment: (fragment, entry),
                    color_formats: &[format],
                    push_constant_bytes: std::mem::size_of::<GlassPush>() as u32,
                    alpha_blend: false,
                    depth_test: None,
                    depth_write: false,
                    name,
                });
                device.destroy_shader_module(fragment);
                pipeline
            };
        let down = make("down_main", "menu glass down", QUARTER_FORMAT);
        let blur = make("blur_main", "menu glass blur", QUARTER_FORMAT);
        let glass = make("glass_main", "menu glass", format);
        device.destroy_shader_module(vertex);
        Ok([down?, blur?, glass?])
    }

    /// Follows the window's format and the HDR interface white (as the HUD does).
    pub(crate) fn set_output(
        &mut self,
        shaders: &ShaderCompiler,
        format: vk::Format,
        ui_white: f32,
    ) -> Result<()> {
        if format != self.format {
            [self.down, self.blur, self.glass] = Self::pipelines(&self.device, shaders, format)?;
            self.format = format;
            self.encoding = crate::overlay::encoding(format);
        }
        self.ui_white = ui_white;
        Ok(())
    }

    /// Declares the glass over `target` (`extent`): the copy, the quarter, the two blurs and the
    /// glass drawn back.
    pub(crate) fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        target: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let small = quarter(extent);
        let copy = graph.transient(TransientDesc {
            name: "menu glass copy",
            width: extent.width,
            height: extent.height,
            format: self.format,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let [a, b] = ["menu glass a", "menu glass b"].map(|name| {
            graph.transient(TransientDesc {
                name,
                width: small.width,
                height: small.height,
                format: QUARTER_FORMAT,
                usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::COLOR_ATTACHMENT,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
            })
        });
        graph
            .pass("app/menu glass copy")
            .image(target, ImageAccess::TransferSrc)
            .image(copy, ImageAccess::TransferDst)
            .run(move |resources, commands| {
                // A copy, not a blit: HDR10's A2R10G10B10 need not support blits.
                commands.copy_image(
                    resources.image(target).raw,
                    resources.image(copy).raw,
                    extent,
                );
                Ok(())
            });
        let texel = |e: vk::Extent2D| [1.0 / e.width as f32, 1.0 / e.height as f32];
        let base = GlassPush {
            image: 0,
            encoding: self.encoding,
            ui_white: self.ui_white,
            axis: 0,
            texel: texel(extent),
            pad: [0.0; 2],
        };
        let steps = [
            ("app/menu glass down", &self.down, copy, a, small, base),
            (
                "app/menu glass blur x",
                &self.blur,
                a,
                b,
                small,
                GlassPush {
                    texel: texel(small),
                    ..base
                },
            ),
            (
                "app/menu glass blur y",
                &self.blur,
                b,
                a,
                small,
                GlassPush {
                    axis: 1,
                    texel: texel(small),
                    ..base
                },
            ),
            ("app/menu glass", &self.glass, a, target, extent, base),
        ];
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        for (name, pipeline, source, out, size, push) in steps {
            graph
                .pass(name)
                .image(source, ImageAccess::Sampled(fragment))
                .image(out, ImageAccess::ColorAttachment)
                .run(move |resources, commands| {
                    let push = GlassPush {
                        image: resources.sampled(source).0,
                        ..push
                    };
                    let color = [vk::RenderingAttachmentInfo::default()
                        .image_view(resources.view(out))
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .load_op(vk::AttachmentLoadOp::DONT_CARE)
                        .store_op(vk::AttachmentStoreOp::STORE)];
                    let info = vk::RenderingInfo::default()
                        .render_area(vk::Rect2D {
                            offset: vk::Offset2D::default(),
                            extent: size,
                        })
                        .layer_count(1)
                        .color_attachments(&color);
                    commands.begin_rendering(&info);
                    commands.bind_pipeline(pipeline);
                    commands.set_viewport_full(size);
                    commands.push_constants(pipeline, &push);
                    commands.draw(3, 1);
                    commands.end_rendering();
                    Ok(())
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quarter_covers_the_frame() {
        let e = |width, height| vk::Extent2D { width, height };
        assert_eq!(quarter(e(2560, 1440)), e(640, 360));
        assert_eq!(quarter(e(1601, 899)), e(401, 225));
        assert_eq!(quarter(e(1, 1)), e(1, 1));
        assert_eq!(std::mem::size_of::<GlassPush>(), 32);
    }
}

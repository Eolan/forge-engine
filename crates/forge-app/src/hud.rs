//! A game's HUD (#328): rectangles over the picture, after its tone mapping and before the debug
//! overlay (`shaders/hud.slang`).
//!
//! The game pushes [`HudQuad`]s into [`crate::Context::hud`] during its update or its render; the
//! shell draws them in their order, blended by their alpha, in the pass `app/hud` (its own zone
//! in the F1 overlay), then empties the list for the next frame. Each is a solid colour or a
//! sampled image of the bindless set (a renderer `TextureSet`'s: a crosshair, a scope's
//! mask) times its colour. `app.hud 0` hides them, for a capture without them.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FullscreenPipelineDesc, ImageAccess,
    ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage,
    TransientDesc, vk,
};

/// The most quads a frame draws; past them the rest are left out (logged once).
pub const MAX_QUADS: usize = 4096;

/// One rectangle of the HUD.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudQuad {
    /// Its corners in the window's pixels: left, top, right, bottom.
    pub rect: [f32; 4],
    /// The region of its texture it shows: u0, v0, u1, v1.
    pub uv: [f32; 4],
    /// A sampled image of the bindless set (its index), or `None` for a solid colour.
    pub texture: Option<u32>,
    /// Its colour, linear with straight alpha, times the texture's.
    pub color: [f32; 4],
}

impl HudQuad {
    /// A rectangle of one colour.
    pub fn solid(rect: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            texture: None,
            color,
        }
    }

    /// A rectangle showing the whole of sampled image `texture`, tinted by `color`.
    pub fn textured(rect: [f32; 4], texture: u32, color: [f32; 4]) -> Self {
        Self {
            rect,
            uv: [0.0, 0.0, 1.0, 1.0],
            texture: Some(texture),
            color,
        }
    }
}

/// A lens over the picture (#328, a scope's): inside a circle, the frame round its centre
/// magnified, tinted and darkened towards its rim. Drawn before the quads, so a reticle stands
/// over it; not in the HDR outputs yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudMagnifier {
    /// Its centre in the window's pixels.
    pub centre: [f32; 2],
    /// Its radius in pixels.
    pub radius: f32,
    /// How much larger the frame shows through it (2 for a 2× scope).
    pub factor: f32,
    /// Times its picture, per channel (a coated lens's slight tint; 1 for none).
    pub tint: [f32; 3],
    /// How dark its rim gets, 0 to 1.
    pub vignette: f32,
    /// Where the darkening starts, a share of the radius.
    pub vignette_start: f32,
}

impl HudMagnifier {
    /// A clear lens of `radius` pixels at `centre`, `factor` times, its rim darkened a little.
    pub fn new(centre: [f32; 2], radius: f32, factor: f32) -> Self {
        Self {
            centre,
            radius,
            factor,
            tint: [1.0; 3],
            vignette: 0.35,
            vignette_start: 0.7,
        }
    }
}

/// Must match `LensPush` in `hud.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LensPush {
    centre: [f32; 2],
    radius: f32,
    factor: f32,
    tint: [f32; 4],
    vignette: f32,
    vignette_start: f32,
    frame: u32,
    encoding: u32,
    size: [f32; 2],
    pad: [f32; 2],
}

/// Must match `Quad` in `hud.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuHudQuad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    texture: u32,
    pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<GpuHudQuad>() == 64);

/// Must match `Push` in `hud.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    quads: u64,
    size: [f32; 2],
    encoding: u32,
    ui_white: f32,
}

/// The HUD's pass and its per-slot quad buffers.
pub(crate) struct Hud {
    device: Arc<Device>,
    pipeline: Pipeline,
    lens: Pipeline,
    format: vk::Format,
    encoding: u32,
    ui_white: f32,
    buffers: Vec<Buffer>,
    warned: std::cell::Cell<bool>,
}

impl Hud {
    /// Compiles the pass for a colour attachment of `format`.
    pub(crate) fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<Self> {
        let buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (MAX_QUADS * std::mem::size_of::<GpuHudQuad>()) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("hud quads {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            device: Arc::clone(device),
            pipeline: Self::pipeline(device, shaders, format)?,
            lens: Self::lens_pipeline(device, shaders, format)?,
            format,
            encoding: crate::overlay::encoding(format),
            ui_white: 203.0,
            buffers,
            warned: std::cell::Cell::new(false),
        })
    }

    fn pipeline(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<Pipeline> {
        let vertex = device.create_shader_module(
            &shaders.compile("hud.slang", "vert_main", ShaderStage::Vertex)?,
            "hud vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("hud.slang", "frag_main", ShaderStage::Fragment)?,
            "hud fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[format],
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            alpha_blend: true,
            depth_test: None,
            depth_write: false,
            name: "hud",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        pipeline
    }

    fn lens_pipeline(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<Pipeline> {
        let vertex = device.create_shader_module(
            &shaders.compile("hud.slang", "lens_vert_main", ShaderStage::Vertex)?,
            "hud lens vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("hud.slang", "lens_main", ShaderStage::Fragment)?,
            "hud lens fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "lens_vert_main"),
            fragment: (fragment, "lens_main"),
            color_formats: &[format],
            push_constant_bytes: std::mem::size_of::<LensPush>() as u32,
            alpha_blend: true,
            depth_test: None,
            depth_write: false,
            name: "hud lens",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        pipeline
    }

    /// Declares the passes of `lens` over `target`: the frame copied (`app/hud lens copy`), then
    /// the lens drawn from the copy (`app/hud lens`).
    pub(crate) fn draw_magnifier<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        target: ImageHandle,
        extent: vk::Extent2D,
        lens: HudMagnifier,
    ) {
        let copy = graph.transient(TransientDesc {
            name: "hud lens copy",
            width: extent.width,
            height: extent.height,
            format: self.format,
            usage: vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        graph
            .pass("app/hud lens copy")
            .image(target, ImageAccess::TransferSrc)
            .image(copy, ImageAccess::TransferDst)
            .run(move |resources, commands| {
                commands.blit_image(
                    resources.image(target).raw,
                    resources.image(copy).raw,
                    extent,
                    vk::Filter::NEAREST,
                );
                Ok(())
            });
        let base = LensPush {
            centre: lens.centre,
            radius: lens.radius,
            factor: lens.factor,
            tint: [lens.tint[0], lens.tint[1], lens.tint[2], 1.0],
            vignette: lens.vignette,
            vignette_start: lens.vignette_start,
            frame: 0,
            encoding: self.encoding,
            size: [extent.width as f32, extent.height as f32],
            pad: [0.0; 2],
        };
        let pipeline = &self.lens;
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        graph
            .pass("app/hud lens")
            .image(copy, ImageAccess::Sampled(fragment))
            .image(target, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let push = LensPush {
                    frame: resources.sampled(copy).0,
                    ..base
                };
                let color = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(target))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&color);
                commands.begin_rendering(&info);
                commands.bind_pipeline(pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(pipeline, &push);
                commands.draw(6, 1);
                commands.end_rendering();
                Ok(())
            });
    }

    /// Follows the window's format and the HDR interface white (as the overlay does).
    pub(crate) fn set_output(
        &mut self,
        shaders: &ShaderCompiler,
        format: vk::Format,
        ui_white: f32,
    ) -> Result<()> {
        if format != self.format {
            self.pipeline = Self::pipeline(&self.device, shaders, format)?;
            self.lens = Self::lens_pipeline(&self.device, shaders, format)?;
            self.format = format;
            self.encoding = crate::overlay::encoding(format);
        }
        self.ui_white = ui_white;
        Ok(())
    }

    /// Declares the pass that draws `quads` over `target`, in their order; nothing for none.
    pub(crate) fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: usize,
        target: ImageHandle,
        extent: vk::Extent2D,
        quads: &[HudQuad],
    ) {
        if quads.is_empty() {
            return;
        }
        if quads.len() > MAX_QUADS && !self.warned.get() {
            tracing::warn!(
                quads = quads.len(),
                "the HUD draws its first {MAX_QUADS} quads a frame"
            );
            self.warned.set(true);
        }
        let records: Vec<GpuHudQuad> = quads
            .iter()
            .take(MAX_QUADS)
            .map(|q| GpuHudQuad {
                rect: q.rect,
                uv: q.uv,
                color: q.color,
                texture: q.texture.unwrap_or(u32::MAX),
                pad: [0; 3],
            })
            .collect();
        let buffer = &self.buffers[slot];
        buffer.write(0, &records);
        let push = Push {
            quads: buffer.address(),
            size: [extent.width as f32, extent.height as f32],
            encoding: self.encoding,
            ui_white: self.ui_white,
        };
        let vertices = records.len() as u32 * 6;
        let pipeline = &self.pipeline;
        graph
            .pass("app/hud")
            .image(target, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let color = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(target))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&color);
                commands.begin_rendering(&info);
                commands.bind_pipeline(pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(pipeline, &push);
                commands.draw(vertices, 1);
                commands.end_rendering();
                Ok(())
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_names_no_texture_as_all_bits() {
        let solid = HudQuad::solid([0.0, 0.0, 4.0, 4.0], [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(solid.texture, None);
        let textured = HudQuad::textured([0.0, 0.0, 4.0, 4.0], 7, [1.0; 4]);
        assert_eq!(textured.texture, Some(7));
        assert_eq!(textured.uv, [0.0, 0.0, 1.0, 1.0]);
    }
}

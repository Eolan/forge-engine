//! Clear glass drawn over the resolved scene (D-065's part 4, the owner's choice of 2026-10-18,
//! `shaders/glass.slang`): the panes of the windows whose rooms are built leave the visibility
//! pass, the resolve shades the room behind them, and the glass is laid over it, seen from the
//! street or from the room.
//!
//! The caller hands over the panes once ([`GlassPanes`]: triangles in the scene's frame, each
//! corner with its room's glow) and, every frame after the sky's compose and the water,
//! [`Glass::draw`] declares two passes:
//! - `glass/requests`: the nearest pane at each pixel asks for its mirror ray, which
//!   [`crate::MeshletRenderer::trace_requested`] traces against the scene as it does the
//!   water's ([`RayRequests`]): the street in the windows from outside, the lit room from
//!   inside at night;
//! - `glass/draw`: every pane over what is behind it, premultiplied: the room dimmed by what the
//!   glass lets through, the sky in the mirror direction by the Fresnel term, the haze in front.
//!
//! The glass writes no depth (TAA reprojects the room through it by the room's own) and is not
//! in the ray-traced scene: the sun, the probes' rays and the mirror rays pass the windows.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot, ImageAccess,
    ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage,
    TransientDesc, VertexPipelineDesc, vk,
};
use glam::{Mat4, Vec3};

use crate::meshlet::RayRequests;
use crate::sky::SkyFrame;
use crate::taa::HDR_FORMAT;

/// The format of the ray requests (the water's).
const REQUEST_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// The panes, as triangles: every three corners one, in the scene's frame.
#[derive(Clone, Debug, Default)]
pub struct GlassPanes {
    /// The corners, metres.
    pub positions: Vec<Vec3>,
    /// Each corner's unit normal (either side is drawn).
    pub normals: Vec<Vec3>,
    /// Each corner's room's glow seen through the pane when the room is lit, cd/m² (zero for a
    /// dark room): what the frame's lamps no longer light farther away ([`GlassLook::glow`]).
    pub glow: Vec<[f32; 3]>,
}

impl GlassPanes {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Whether there is no pane.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// How the glass looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassLook {
    /// The share of the light the glass lets through besides what its surfaces reflect: a
    /// clean pane's absorption and its second surface.
    pub transmission: f32,
    /// The share reflected at normal incidence (Fresnel's F0), rising towards 1 at grazing
    /// angles.
    pub reflectance: f32,
    /// Metres from the camera over which a lit room's glow fades in ([`GlassPanes::glow`]),
    /// where the frame's lamps ([`crate::lamps::nearest`]) stop lighting the rooms.
    pub glow: [f32; 2],
}

impl Default for GlassLook {
    fn default() -> Self {
        Self {
            transmission: 0.92,
            reflectance: 0.04,
            glow: [30.0, 60.0],
        }
    }
}

/// What a frame's glass needs.
#[derive(Clone, Copy, Debug)]
pub struct GlassParams {
    /// The drawing camera's view-projection, camera-relative (jitter included).
    pub view_proj: Mat4,
    /// The camera in the scene's frame.
    pub camera: Vec3,
    /// Pre-exposed luminance of a unit of the sky's table (the sun's illuminance times the
    /// exposure).
    pub sky_scale: f32,
    /// Pre-exposed units per cd/m².
    pub exposure: f32,
}

/// Must match `GlassVertex` in `glass.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuGlassVertex {
    position: [f32; 4],
    normal: [f32; 4],
    glow: [f32; 4],
}

/// Must match `GlassFrame` in `glass.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuGlassFrame {
    view_proj: [f32; 16],
    camera: [f32; 4],
    look: [f32; 4],
    sky_scale: f32,
    exposure: f32,
    depth: u32,
    pad: u32,
    width: f32,
    height: f32,
    pad2: [u32; 2],
    vertices: u64,
    sky: u64,
    sky_light: u64,
    pad4: u64,
}

const _: () = assert!(std::mem::size_of::<GpuGlassFrame>() == 160);

/// The panes and their passes (D-065's part 4).
pub struct Glass {
    /// What a frame draws them with.
    pub look: GlassLook,
    vertices: Buffer,
    vertex_count: u32,
    requests: Pipeline,
    draw: Pipeline,
    /// Per frame slot: the frame's block, written by the CPU.
    frames: Vec<Buffer>,
}

impl Glass {
    /// Compiles the passes and uploads `panes`.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler, panes: &GlassPanes) -> Result<Self> {
        let module = |entry: &str, stage, name: &str| -> Result<vk::ShaderModule> {
            device.create_shader_module(&shaders.compile("glass.slang", entry, stage)?, name)
        };
        let vertex = module("vert_main", ShaderStage::Vertex, "glass vertex")?;
        let request_fragment = module("request_main", ShaderStage::Fragment, "glass requests")?;
        let draw_fragment = module("draw_main", ShaderStage::Fragment, "glass draw")?;
        let push = std::mem::size_of::<u64>() as u32;
        let requests = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (request_fragment, "request_main"),
            color_formats: &[REQUEST_FORMAT, REQUEST_FORMAT],
            depth_format: Some(vk::Format::D32_SFLOAT),
            push_constant_bytes: push,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            // The panes' own depth: the nearest one's ray.
            depth_test: true,
            alpha_blend: false,
            premultiplied: false,
            name: "glass requests",
        });
        let draw = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (draw_fragment, "draw_main"),
            color_formats: &[HDR_FORMAT],
            depth_format: None,
            push_constant_bytes: push,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: false,
            alpha_blend: true,
            premultiplied: true,
            name: "glass draw",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(request_fragment);
        device.destroy_shader_module(draw_fragment);
        let corners: Vec<GpuGlassVertex> = panes
            .positions
            .iter()
            .zip(&panes.normals)
            .zip(&panes.glow)
            .map(|((p, n), g)| GpuGlassVertex {
                position: p.extend(1.0).to_array(),
                normal: n.extend(0.0).to_array(),
                glow: [g[0], g[1], g[2], 0.0],
            })
            .collect();
        let vertices = device.create_buffer_with_data(
            &corners,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Geometry,
            "glass panes",
        )?;
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuGlassFrame>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("glass frame {i}"),
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            look: GlassLook::default(),
            vertices,
            vertex_count: (corners.len() / 3 * 3) as u32,
            requests: requests?,
            draw: draw?,
            frames,
        })
    }

    /// Triangles drawn.
    pub fn triangle_count(&self) -> u32 {
        self.vertex_count / 3
    }

    /// Bytes of the panes.
    pub fn bytes(&self) -> u64 {
        self.vertices.size()
    }

    /// Declares the passes over `color` (the frame's HDR image, after the sky's compose and the
    /// water) against `depth` (the frame's): the nearest panes' mirror rays asked for, then
    /// every pane drawn. Returns the requests for
    /// [`crate::MeshletRenderer::trace_requested`].
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        sky: &SkyFrame,
        params: GlassParams,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
    ) -> RayRequests {
        let image = |name, format, usage, aspect| TransientDesc {
            name,
            width: extent.width,
            height: extent.height,
            format,
            usage: usage | vk::ImageUsageFlags::SAMPLED,
            aspect,
            mip_levels: 1,
        };
        let request = |name| {
            image(
                name,
                REQUEST_FORMAT,
                vk::ImageUsageFlags::COLOR_ATTACHMENT,
                vk::ImageAspectFlags::COLOR,
            )
        };
        let mirror = graph.transient(request("glass mirror ray requests"));
        let sun = graph.transient(request("glass shadow ray requests"));
        let glass_depth = graph.transient(image(
            "glass depth",
            vk::Format::D32_SFLOAT,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            vk::ImageAspectFlags::DEPTH,
        ));
        let frame_buffer = &self.frames[slot.index];
        let gpu = GpuGlassFrame {
            view_proj: params.view_proj.to_cols_array(),
            camera: params.camera.extend(0.0).to_array(),
            look: [
                self.look.transmission,
                self.look.reflectance,
                self.look.glow[0],
                self.look.glow[1],
            ],
            sky_scale: params.sky_scale,
            exposure: params.exposure,
            depth: 0,
            pad: 0,
            width: extent.width as f32,
            height: extent.height as f32,
            pad2: [0; 2],
            vertices: self.vertices.address(),
            sky: sky.address(),
            sky_light: sky.light.address,
            pad4: 0,
        };
        frame_buffer.write(0, &[gpu]);
        // The frame's depth by its sampled handle, known once the graph runs.
        let set_depth = move |resources: &forge_gpu::Resources<'_>| {
            frame_buffer.write(
                std::mem::offset_of!(GpuGlassFrame, depth) as u64,
                &[resources.sampled(depth).0],
            );
        };
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        let count = self.vertex_count;
        let (requests, draw) = (&self.requests, &self.draw);
        let address = frame_buffer.address();
        graph
            .pass("glass/requests")
            .image(depth, ImageAccess::Sampled(fragment))
            .image(sky.light.table, ImageAccess::Sampled(fragment))
            .image(sky.aerial(), ImageAccess::Sampled(fragment))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(fragment))
            .image(mirror, ImageAccess::ColorAttachment)
            .image(sun, ImageAccess::ColorAttachment)
            .image(glass_depth, ImageAccess::DepthAttachment)
            .run(move |resources, commands| {
                set_depth(resources);
                let cleared = |image| {
                    vk::RenderingAttachmentInfo::default()
                        .image_view(resources.view(image))
                        .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                        .load_op(vk::AttachmentLoadOp::CLEAR)
                        .store_op(vk::AttachmentStoreOp::STORE)
                        .clear_value(vk::ClearValue::default())
                };
                let attachments = [cleared(mirror), cleared(sun)];
                // Reversed-Z: cleared to the far plane.
                let depth_attachment = vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(glass_depth))
                    .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .clear_value(vk::ClearValue::default());
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments)
                    .depth_attachment(&depth_attachment);
                commands.begin_rendering(&info);
                commands.bind_pipeline(requests);
                commands.set_viewport_full(extent);
                commands.push_constants(requests, &address);
                commands.draw(count, 1);
                commands.end_rendering();
                Ok(())
            });
        graph
            .pass("glass/draw")
            .image(depth, ImageAccess::Sampled(fragment))
            .image(sky.light.table, ImageAccess::Sampled(fragment))
            .image(sky.aerial(), ImageAccess::Sampled(fragment))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(fragment))
            .image(color, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                set_depth(resources);
                let attachments = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(color))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments);
                commands.begin_rendering(&info);
                commands.bind_pipeline(draw);
                commands.set_viewport_full(extent);
                commands.push_constants(draw, &address);
                commands.draw(count, 1);
                commands.end_rendering();
                Ok(())
            });
        RayRequests {
            mirror,
            sun,
            depth: glass_depth,
            view_proj: params.view_proj,
        }
    }
}

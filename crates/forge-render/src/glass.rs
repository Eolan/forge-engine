//! Clear glass drawn over the resolved scene (D-065's part 4, the owner's choice of 2026-10-18,
//! `shaders/glass.slang`): the panes of the windows whose rooms are built leave the visibility
//! pass, the resolve shades the room behind them, and the glass is laid over it, seen from the
//! street or from the room.
//!
//! The caller hands over the panes once ([`GlassPanes`]: triangles in the scene's frame, each
//! corner with its room's glow and its pane's id), moves or hides a pane by its id when the game
//! says ([`Glass::set_pane`], #325: a sash raised, a window smashed) and, every frame after the
//! sky's compose and the water,
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
use std::sync::atomic::{AtomicU64, Ordering};

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

/// A pane corner's [`GlassPanes::lamp`] when its glow fades in by distance.
pub const NO_LAMP: u32 = u32::MAX;

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
    /// Each corner's room's lamp (#284): an index into the lamps the frames light with
    /// ([`crate::MeshletScene::set_lamps`]) whose share the frame leaves unlit
    /// ([`Glass::set_unlit`]) lights the glow, so a room shows its glow exactly where the frame's
    /// lamps no longer light it; [`NO_LAMP`] (or none given) for a glow faded in by distance
    /// ([`GlassLook::glow`]).
    pub lamp: Vec<u32>,
    /// Each corner's pane (#325): the id, from 0, by which the host moves or hides it at run
    /// time ([`Glass::set_pane`]: a sash raised, a window smashed); none given, every corner is
    /// pane 0.
    pub pane: Vec<u32>,
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
    /// Metres from the camera over which a lit room's glow fades in on a pane without a room lamp
    /// ([`GlassPanes::glow`], [`GlassPanes::lamp`]),
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
    /// Towards the sun, unit (#339's glints).
    pub sun: Vec3,
    /// The sun's pre-exposed illuminance through the air, per channel: 0 below the horizon and
    /// at night.
    pub sun_light: Vec3,
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
    unlit_count: u32,
    width: f32,
    height: f32,
    panes: u64,
    vertices: u64,
    sky: u64,
    sky_light: u64,
    unlit: u64,
    sun: [f32; 4],
    sun_light: [f32; 4],
    pane_heights: u64,
}

const _: () = assert!(std::mem::size_of::<GpuGlassFrame>() == 200);

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
    /// Per frame slot: the share of each lamp the frame leaves unlit ([`Self::set_unlit`]).
    unlit: Vec<Buffer>,
    /// How many lamps those hold: the panes' greatest lamp index and one.
    unlit_count: u32,
    /// Each pane's transform (#325): three rows of a 3 × 4 matrix from the corners as given to
    /// where they are drawn; all zero for a hidden pane.
    panes: Vec<[f32; 12]>,
    /// Per frame slot: the panes' transforms as that slot's frames read them.
    pane_buffers: Vec<Buffer>,
    /// Each pane's lowest and highest corner as given (#339's blinds), when the panes have ids.
    heights: Option<Buffer>,
    /// The panes' edits so far, and per frame slot how many its buffer holds.
    edits: u64,
    written: Vec<AtomicU64>,
}

/// The identity as a pane's transform.
const PANE_IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

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
            .enumerate()
            .map(|(i, ((p, n), g))| GpuGlassVertex {
                position: p.extend(1.0).to_array(),
                normal: n
                    .extend(f32::from_bits(panes.pane.get(i).copied().unwrap_or(0)))
                    .to_array(),
                glow: [
                    g[0],
                    g[1],
                    g[2],
                    f32::from_bits(panes.lamp.get(i).copied().unwrap_or(NO_LAMP)),
                ],
            })
            .collect();
        let unlit_count = panes
            .lamp
            .iter()
            .filter(|&&l| l != NO_LAMP)
            .max()
            .map_or(0, |&l| l + 1);
        let unlit = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                let buffer = device.create_buffer(BufferDesc {
                    size: u64::from(unlit_count.max(4)) * 4,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("glass unlit lamps {i}"),
                })?;
                // Until a frame says otherwise, every lamp's light is the glow's.
                buffer.write(0, &vec![1.0_f32; unlit_count as usize]);
                Ok(buffer)
            })
            .collect::<Result<_>>()?;
        let vertices = device.create_buffer_with_data(
            &corners,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Geometry,
            "glass panes",
        )?;
        let pane_count = panes.pane.iter().max().map_or(1, |&p| p as usize + 1);
        // Each pane's height span, for the blinds some panes have (#339): only when the host
        // gave the panes ids (one pane holding every corner has no span of its own).
        let heights = (pane_count > 1)
            .then(|| {
                let mut spans = vec![[f32::MAX, f32::MIN]; pane_count];
                for (p, id) in panes.positions.iter().zip(&panes.pane) {
                    let span = &mut spans[*id as usize];
                    span[0] = span[0].min(p.y);
                    span[1] = span[1].max(p.y);
                }
                device.create_buffer_with_data(
                    &spans,
                    vk::BufferUsageFlags::STORAGE_BUFFER,
                    MemoryCategory::Geometry,
                    "glass pane heights",
                )
            })
            .transpose()?;
        let pane_rows = vec![PANE_IDENTITY; pane_count];
        let pane_buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                let buffer = device.create_buffer(BufferDesc {
                    size: std::mem::size_of_val(pane_rows.as_slice()) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("glass pane transforms {i}"),
                })?;
                buffer.write(0, &pane_rows);
                Ok(buffer)
            })
            .collect::<Result<_>>()?;
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
            unlit,
            unlit_count,
            panes: pane_rows,
            pane_buffers,
            heights,
            edits: 0,
            written: (0..FRAMES_IN_FLIGHT).map(|_| AtomicU64::new(0)).collect(),
        })
    }

    /// How many panes: the greatest id in [`GlassPanes::pane`] and one.
    pub fn pane_count(&self) -> u32 {
        self.panes.len() as u32
    }

    /// Moves pane `id` (#325) by `transform` from where its corners were given (the identity puts
    /// it back), or hides it (`None`: a window smashed). The next frames draw it so, its mirror
    /// rays and its glow following; an id past [`Self::pane_count`] changes nothing.
    pub fn set_pane(&mut self, id: u32, transform: Option<Mat4>) {
        let Some(rows) = self.panes.get_mut(id as usize) else {
            return;
        };
        *rows = transform.map_or([0.0; 12], |m| {
            let r = m.transpose();
            let mut out = [0.0; 12];
            out[..4].copy_from_slice(&r.x_axis.to_array());
            out[4..8].copy_from_slice(&r.y_axis.to_array());
            out[8..].copy_from_slice(&r.z_axis.to_array());
            out
        });
        self.edits += 1;
    }

    /// The share of each lamp's light frame `slot` leaves unlit ([`crate::lamps::unlit`]), by
    /// the lamps' indices in the frame's list: a pane whose room's lamp it is
    /// ([`GlassPanes::lamp`]) glows with that share of its room's light. A pane's lamp past the
    /// end of `unlit` is taken as unlit.
    pub fn set_unlit(&self, slot: FrameSlot, unlit: &[f32]) {
        let mut shares = vec![1.0_f32; self.unlit_count as usize];
        for (s, &u) in shares.iter_mut().zip(unlit) {
            *s = u;
        }
        if !shares.is_empty() {
            self.unlit[slot.index].write(0, &shares);
        }
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
        // The panes' transforms, into this slot's copy when edits came since it was written.
        let panes = &self.pane_buffers[slot.index];
        if self.written[slot.index].load(Ordering::Relaxed) != self.edits {
            panes.write(0, &self.panes);
            self.written[slot.index].store(self.edits, Ordering::Relaxed);
        }
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
            unlit_count: self.unlit_count,
            width: extent.width as f32,
            height: extent.height as f32,
            panes: panes.address(),
            vertices: self.vertices.address(),
            sky: sky.address(),
            sky_light: sky.light.address,
            unlit: self.unlit[slot.index].address(),
            sun: params.sun.extend(0.0).to_array(),
            sun_light: params.sun_light.extend(0.0).to_array(),
            pane_heights: self.heights.as_ref().map_or(0, Buffer::address),
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

//! Light with no surface, laid over the scene (`shaders/holograms.slang`): a floating sign, a
//! price over a stall, a picture on a fridge's door. A scene whose meshes carry such pictures
//! leaves them out of the visibility pass and hands them over once ([`HologramPanes`]: triangles
//! in the scene's frame, each corner naming its [`HologramRow`]), and every frame after the glass,
//! [`Holograms::draw`] adds each picture's light over what is behind it, tested against the frame's
//! depth and writing none.
//!
//! No surface: a hologram takes no light, casts no shadow, is not in the ray-traced scene and lights
//! nothing. What is behind it shows through unchanged. Because it writes no depth, the upscaler
//! reprojects the pixel by what is behind, so a picture over something near it may trail a little
//! when the camera moves fast. The glass writes no depth either, so a picture behind a pane isn't
//! dimmed by it, and the sky's haze isn't laid over a far one (the froxels' fog is).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot, GpuError, ImageAccess,
    ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage,
    VertexPipelineDesc, vk,
};
use glam::{Mat4, Vec3};

use crate::taa::HDR_FORMAT;

/// A picture and what is done to it.
#[derive(Clone, Copy, Debug)]
pub struct HologramRow {
    /// The picture, a sampled image's index in the bindless set
    /// ([`crate::material::TextureSet::sampled`]). Its colour times its alpha is the light it adds.
    pub picture: u32,
    /// What a full white of the picture adds, cd/m².
    pub glow: [f32; 3],
    /// Turns of the picture's hue, one being all the way round.
    pub hue: f32,
    /// The picture's saturation, 1 unchanged.
    pub saturation: f32,
    /// The picture's value, 1 unchanged.
    pub value: f32,
}

/// The holograms, as triangles: every three corners one.
#[derive(Clone, Debug, Default)]
pub struct HologramPanes {
    /// The corners, metres: in the world frame from a scenario, then in the scene's.
    pub positions: Vec<Vec3>,
    /// Each corner's place in its picture.
    pub uvs: Vec<[f32; 2]>,
    /// Each corner's row in `rows`.
    pub row: Vec<u32>,
    /// The pictures.
    pub rows: Vec<HologramRow>,
}

impl HologramPanes {
    /// How many triangles.
    pub fn triangle_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// A frame's view of them.
#[derive(Clone, Copy, Debug)]
pub struct HologramParams {
    /// From the camera-relative scene to the clip space (with the frame's jitter).
    pub view_proj: Mat4,
    /// The camera in the scene's frame.
    pub camera: Vec3,
    /// What a cd/m² becomes in the frame.
    pub exposure: f32,
}

/// Must match `HologramVertex` in `holograms.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuVertex {
    position: [f32; 3],
    row: u32,
    uv: [f32; 2],
    pad: [f32; 2],
}

/// Must match `HologramRow` in `holograms.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuRow {
    glow: [f32; 3],
    hue: f32,
    picture: u32,
    saturation: f32,
    value: f32,
    pad: f32,
}

/// Must match `HologramFrame` in `holograms.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuFrame {
    view_proj: [f32; 16],
    camera: [f32; 3],
    exposure: f32,
    vertices: u64,
    rows: u64,
    depth: u32,
    pad: [u32; 3],
}

/// The pass and its buffers.
pub struct Holograms {
    draw: Pipeline,
    vertices: Buffer,
    rows: Buffer,
    frames: Vec<Buffer>,
    vertex_count: u32,
}

impl Holograms {
    /// The pass over `panes`, which are in the scene's frame. Refused where a corner lacks its place
    /// in the picture or names a row there isn't: the shader would read past the rows' end.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        panes: &HologramPanes,
    ) -> Result<Self> {
        let corners = panes.positions.len();
        if panes.uvs.len() != corners
            || panes.row.len() != corners
            || !corners.is_multiple_of(3)
            || panes.row.iter().any(|r| *r as usize >= panes.rows.len())
        {
            return Err(GpuError::Unsupported(format!(
                "holograms: {corners} corners, {} places, {} row names, {} rows",
                panes.uvs.len(),
                panes.row.len(),
                panes.rows.len()
            )));
        }
        let vertex_data: Vec<GpuVertex> = panes
            .positions
            .iter()
            .zip(&panes.uvs)
            .zip(&panes.row)
            .map(|((p, uv), row)| GpuVertex {
                position: p.to_array(),
                row: *row,
                uv: *uv,
                pad: [0.0; 2],
            })
            .collect();
        let row_data: Vec<GpuRow> = panes
            .rows
            .iter()
            .map(|r| GpuRow {
                glow: r.glow,
                hue: r.hue,
                picture: r.picture,
                saturation: r.saturation,
                value: r.value,
                pad: 0.0,
            })
            .collect();
        let shared = |name, bytes: u64| {
            device.create_buffer(BufferDesc {
                size: bytes.max(16),
                usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                location: MemoryLocation::CpuToGpu,
                category: MemoryCategory::Geometry,
                name,
            })
        };
        let vertices = shared(
            "hologram vertices",
            std::mem::size_of_val(vertex_data.as_slice()) as u64,
        )?;
        vertices.write(0, &vertex_data);
        let rows = shared(
            "hologram rows",
            std::mem::size_of_val(row_data.as_slice()) as u64,
        )?;
        rows.write(0, &row_data);
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|_| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuFrame>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: "hologram frame",
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let vertex = device.create_shader_module(
            &shaders.compile("holograms.slang", "vert_main", ShaderStage::Vertex)?,
            "holograms vertex",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("holograms.slang", "frag_main", ShaderStage::Fragment)?,
            "holograms fragment",
        )?;
        let draw = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[HDR_FORMAT],
            depth_format: None,
            push_constant_bytes: std::mem::size_of::<u64>() as u32,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: false,
            // Premultiplied with no coverage: light added.
            alpha_blend: true,
            premultiplied: true,
            name: "holograms",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        let draw = draw?;
        Ok(Self {
            draw,
            vertices,
            rows,
            frames,
            vertex_count: vertex_data.len() as u32,
        })
    }

    /// How many triangles it draws.
    pub fn triangle_count(&self) -> u32 {
        self.vertex_count / 3
    }

    /// The bytes it keeps on the GPU.
    pub fn bytes(&self) -> u64 {
        self.vertices.size() + self.rows.size()
    }

    /// Adds their light over `color` (the HDR scene, pre-exposed), behind the frame's `depth`.
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        params: HologramParams,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let frame_buffer = &self.frames[slot.index];
        frame_buffer.write(
            0,
            &[GpuFrame {
                view_proj: params.view_proj.to_cols_array(),
                camera: params.camera.to_array(),
                exposure: params.exposure,
                vertices: self.vertices.address(),
                rows: self.rows.address(),
                depth: 0,
                pad: [0; 3],
            }],
        );
        let address = frame_buffer.address();
        let count = self.vertex_count;
        let draw = &self.draw;
        let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
        graph
            .pass("holograms/draw")
            .image(depth, ImageAccess::Sampled(fragment))
            .image(color, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                frame_buffer.write(
                    std::mem::offset_of!(GpuFrame, depth) as u64,
                    &[resources.sampled(depth).0],
                );
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
    }
}

//! The mip check (issue #20, `shaders/mipcheck.slang`): the visibility resolve samples
//! textures with derivatives it reconstructs from the triangle (`SampleGrad`), where a
//! fragment shader would take them from its 2×2 quad. This check draws a ground quad at a
//! grazing angle both ways, samples a mip ramp (level k holds k / 16) trilinearly so that
//! each sample is the level of detail the sampler chose, and reports how far the two choices
//! are apart. `meshlets --mip-check` runs it; the resolve's texturing is right when every
//! pixel is within one level.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, ImageAccess, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, TransientDesc, VertexPipelineDesc, vk,
};
use glam::{Vec3, Vec4};

use crate::material::TextureSet;
use crate::textures;

/// Mirrors `Params` in `mipcheck.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view_proj: [f32; 16],
    corners: [[f32; 4]; 4],
    ramp: u32,
    reference: u32,
    analytic: u32,
    width: u32,
    height: u32,
    uv_scale: f32,
    analytic_sampled: u32,
    pad: u32,
    result: u64,
}

/// How far the two passes' levels of detail are apart, over the pixels both covered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MipCheckResult {
    /// The largest difference, in mip levels.
    pub max_levels: f32,
    /// The mean difference, in mip levels.
    pub mean_levels: f32,
    /// Pixels compared.
    pub pixels: u32,
}

impl MipCheckResult {
    /// Whether every pixel chose a level within one of the fragment shader's.
    pub fn passes(&self) -> bool {
        self.pixels > 0 && self.max_levels < 1.0
    }
}

/// The check's pipelines, its ramp texture and its result buffer.
pub struct MipCheck {
    reference: Pipeline,
    analytic: Pipeline,
    compare: Pipeline,
    ramp: TextureSet,
    /// Per frame slot: the parameters (host-written while the frame is recorded).
    params: Vec<Buffer>,
    /// Max difference (float bits), pixels, sum (1/1024 levels) of the last frame.
    result: GraphBuffer,
}

impl MipCheck {
    /// Compiles the passes and uploads the ramp (1024 × 1024, 11 levels).
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("mipcheck.slang", entry, ShaderStage::Compute)?,
                entry,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: 8,
                name: entry,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let vertex = device.create_shader_module(
            &shaders.compile("mipcheck.slang", "reference_vert", ShaderStage::Vertex)?,
            "mip check vertex",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("mipcheck.slang", "reference_frag", ShaderStage::Fragment)?,
            "mip check fragment",
        )?;
        let reference = device.create_vertex_pipeline(&VertexPipelineDesc {
            vertex: (vertex, "reference_vert"),
            fragment: (fragment, "reference_frag"),
            color_formats: &[vk::Format::R16G16B16A16_SFLOAT],
            depth_format: None,
            push_constant_bytes: 8,
            cull_mode: vk::CullModeFlags::NONE,
            wireframe: false,
            depth_test: false,
            name: "mip check reference",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        let mut ramp = TextureSet::new(device);
        ramp.add(&textures::mip_ramp(1024))?;
        let params = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<Params>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("mip check parameters {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let result = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: 16,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name: "mip check result",
        })?);
        Ok(Self {
            reference: reference?,
            analytic: compute("analytic_main")?,
            compare: compute("compare_main")?,
            ramp,
            params,
            result,
        })
    }

    /// Declares the check's passes for a target of `extent`: the reference draw, the
    /// analytic pass and the compare (whose result [`MipCheck::result`] reads).
    pub fn record<'f>(&'f self, graph: &mut FrameGraph<'f>, slot: FrameSlot, extent: vk::Extent2D) {
        let image = |name, usage| TransientDesc {
            name,
            width: extent.width,
            height: extent.height,
            format: vk::Format::R16G16B16A16_SFLOAT,
            usage: usage | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        };
        let reference = graph.transient(image(
            "mip check reference",
            vk::ImageUsageFlags::COLOR_ATTACHMENT,
        ));
        let analytic = graph.transient(image("mip check analytic", vk::ImageUsageFlags::STORAGE));
        let result = graph.import_buffer(&self.result);
        let params: &'f Buffer = &self.params[slot.index];
        let result_buffer: &'f GraphBuffer = &self.result;
        let ramp = self.ramp.sampled(forge_core::TextureId(0));
        // A 160 m by 400 m ground seen from 1.6 m up, looking down the quad's length.
        let aspect = extent.width as f32 / extent.height.max(1) as f32;
        let view = glam::camera::rh::view::look_at_mat4(
            Vec3::new(0.0, 1.6, 0.0),
            Vec3::new(0.0, 0.4, -20.0),
            Vec3::Y,
        );
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(
            60f32.to_radians(),
            aspect,
            0.1,
        );
        let corners = [
            Vec4::new(-80.0, 0.0, -0.5, 1.0),
            Vec4::new(80.0, 0.0, -0.5, 1.0),
            Vec4::new(80.0, 0.0, -400.0, 1.0),
            Vec4::new(-80.0, 0.0, -400.0, 1.0),
        ];
        let write_params = move |resources: &forge_gpu::Resources<'_>| {
            params.write(
                0,
                &[Params {
                    view_proj: (proj * view).to_cols_array(),
                    corners: corners.map(|c| c.to_array()),
                    ramp,
                    reference: resources.sampled(reference).0,
                    analytic: resources.storage(analytic, 0).0,
                    width: extent.width,
                    height: extent.height,
                    uv_scale: 0.5,
                    analytic_sampled: resources.sampled(analytic).0,
                    pad: 0,
                    result: result_buffer.address(),
                }],
            );
            params.address()
        };
        let reference_pipeline = &self.reference;
        graph
            .pass("check/mip reference")
            .image(reference, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let address = write_params(resources);
                let attachments = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(reference))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::CLEAR)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments);
                commands.begin_rendering(&info);
                commands.bind_pipeline(reference_pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(reference_pipeline, &address);
                commands.draw(6, 1);
                commands.end_rendering();
                Ok(())
            });
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let analytic_pipeline = &self.analytic;
        graph
            .pass("check/mip analytic")
            .image(analytic, ImageAccess::StorageWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(analytic_pipeline);
                commands.push_constants(analytic_pipeline, &params.address());
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });
        graph
            .pass("check/mip compare")
            .buffer(result, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(result_buffer, 0, 16, 0);
                Ok(())
            });
        let compare_pipeline = &self.compare;
        graph
            .pass("check/mip compare")
            .image(reference, ImageAccess::Sampled(compute))
            .image(analytic, ImageAccess::Sampled(compute))
            .buffer(result, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(compare_pipeline);
                commands.push_constants(compare_pipeline, &params.address());
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });
        graph
            .pass("check/mip compare")
            .buffer(result, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// The last completed check (call once the device is idle).
    pub fn result(&self) -> MipCheckResult {
        let mut words = [0_u32; 4];
        self.result.read(0, &mut words);
        MipCheckResult {
            max_levels: f32::from_bits(words[0]),
            mean_levels: words[2] as f32 / 1024.0 / words[1].max(1) as f32,
            pixels: words[1],
        }
    }
}

use std::ffi::CString;
use std::sync::Arc;

use ash::vk;

use crate::device::Device;
use crate::error::Result;

/// A pipeline and its layout. Destroyed on drop (the GPU must be done with it).
pub struct Pipeline {
    device: Arc<Device>,
    raw: vk::Pipeline,
    layout: vk::PipelineLayout,
    bind_point: vk::PipelineBindPoint,
    push_constant_stages: vk::ShaderStageFlags,
}

impl Pipeline {
    /// The pipeline.
    pub fn raw(&self) -> vk::Pipeline {
        self.raw
    }
    /// Its layout (push constants only in this engine's pointer-based model).
    pub fn layout(&self) -> vk::PipelineLayout {
        self.layout
    }
    /// Bind point.
    pub fn bind_point(&self) -> vk::PipelineBindPoint {
        self.bind_point
    }
    /// Stages that see the push constants.
    pub fn push_constant_stages(&self) -> vk::ShaderStageFlags {
        self.push_constant_stages
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        // SAFETY: the owner guarantees no command buffer in flight uses the pipeline.
        unsafe {
            self.device.raw().destroy_pipeline(self.raw, None);
            self.device.raw().destroy_pipeline_layout(self.layout, None);
        }
    }
}

/// Description of a task/mesh/fragment pipeline rendering with dynamic rendering.
pub struct MeshPipelineDesc<'a> {
    /// Optional task stage: module and entry point name.
    pub task: Option<(vk::ShaderModule, &'a str)>,
    /// Mesh stage.
    pub mesh: (vk::ShaderModule, &'a str),
    /// Fragment stage.
    pub fragment: (vk::ShaderModule, &'a str),
    /// Color attachment formats.
    pub color_formats: &'a [vk::Format],
    /// Depth attachment format, if any.
    pub depth_format: Option<vk::Format>,
    /// Push constant size in bytes (visible to all stages).
    pub push_constant_bytes: u32,
    /// Face culling.
    pub cull_mode: vk::CullModeFlags,
    /// Wireframe rasterisation.
    pub wireframe: bool,
    /// Depth test with reversed-Z (`GREATER_OR_EQUAL`) and write.
    pub depth_test: bool,
    /// Debug name.
    pub name: &'a str,
}

/// Description of a full-screen vertex + fragment pipeline (no vertex input, no depth).
pub struct FullscreenPipelineDesc<'a> {
    /// Vertex stage (typically a 3-vertex triangle from `SV_VertexID`).
    pub vertex: (vk::ShaderModule, &'a str),
    /// Fragment stage.
    pub fragment: (vk::ShaderModule, &'a str),
    /// Colour attachment formats.
    pub color_formats: &'a [vk::Format],
    /// Push constant size in bytes.
    pub push_constant_bytes: u32,
    /// Blend the output over the attachment with its alpha (overlays); opaque otherwise.
    pub alpha_blend: bool,
    /// Test against a depth attachment of this format without writing it (reversed-Z
    /// `GREATER_OR_EQUAL`: a full-screen triangle at depth 0 then covers only the pixels
    /// nothing was drawn to, which is how the sky is drawn last).
    pub depth_test: Option<vk::Format>,
    /// Debug name.
    pub name: &'a str,
}

/// Description of a compute pipeline.
pub struct ComputePipelineDesc<'a> {
    /// Module and entry point.
    pub shader: (vk::ShaderModule, &'a str),
    /// Push constant size in bytes.
    pub push_constant_bytes: u32,
    /// Debug name.
    pub name: &'a str,
}

impl Device {
    /// Pipeline layout = the bindless set + one push-constant range for `stages`.
    fn create_layout(
        &self,
        stages: vk::ShaderStageFlags,
        push_constant_bytes: u32,
    ) -> Result<vk::PipelineLayout> {
        let ranges = [vk::PushConstantRange {
            stage_flags: stages,
            offset: 0,
            size: push_constant_bytes.max(4),
        }];
        let set_layouts = [self.bindless_layout()];
        let layout_info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&set_layouts)
            .push_constant_ranges(&ranges);
        // SAFETY: valid layout info; the bindless layout lives as long as the device.
        Ok(unsafe { self.raw().create_pipeline_layout(&layout_info, None)? })
    }

    /// Creates a compute pipeline.
    pub fn create_compute_pipeline(
        self: &Arc<Self>,
        desc: &ComputePipelineDesc<'_>,
    ) -> Result<Pipeline> {
        let layout = self.create_layout(vk::ShaderStageFlags::COMPUTE, desc.push_constant_bytes)?;
        let name = CString::new(desc.shader.1).expect("entry point name");
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(desc.shader.0)
            .name(&name);
        let info = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(layout);
        // SAFETY: valid create info; the module outlives the call.
        let created = unsafe {
            self.raw()
                .create_compute_pipelines(vk::PipelineCache::null(), &[info], None)
        };
        let raw = match created {
            Ok(pipelines) => pipelines[0],
            Err((_, e)) => {
                // SAFETY: the layout is unused by any pipeline.
                unsafe { self.raw().destroy_pipeline_layout(layout, None) };
                return Err(e.into());
            }
        };
        self.set_name(raw, desc.name);
        Ok(Pipeline {
            device: Arc::clone(self),
            raw,
            layout,
            bind_point: vk::PipelineBindPoint::COMPUTE,
            push_constant_stages: vk::ShaderStageFlags::COMPUTE,
        })
    }

    /// Creates a full-screen pipeline: triangle list, no vertex input, no depth, no culling.
    pub fn create_fullscreen_pipeline(
        self: &Arc<Self>,
        desc: &FullscreenPipelineDesc<'_>,
    ) -> Result<Pipeline> {
        let stages_mask = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
        let layout = self.create_layout(stages_mask, desc.push_constant_bytes)?;
        let vertex_name = CString::new(desc.vertex.1).expect("entry point name");
        let fragment_name = CString::new(desc.fragment.1).expect("entry point name");
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(desc.vertex.0)
                .name(&vertex_name),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(desc.fragment.0)
                .name(&fragment_name),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachments: Vec<_> = desc
            .color_formats
            .iter()
            .map(|_| {
                vk::PipelineColorBlendAttachmentState::default()
                    .color_write_mask(vk::ColorComponentFlags::RGBA)
                    .blend_enable(desc.alpha_blend)
                    .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                    .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                    .color_blend_op(vk::BlendOp::ADD)
                    .src_alpha_blend_factor(vk::BlendFactor::ONE)
                    .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
                    .alpha_blend_op(vk::BlendOp::ADD)
            })
            .collect();
        let blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let depth_stencil = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(desc.depth_test.is_some())
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL);
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(desc.color_formats)
            .depth_attachment_format(desc.depth_test.unwrap_or(vk::Format::UNDEFINED));
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_stencil)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .push_next(&mut rendering);
        // SAFETY: all referenced state lives until the call returns.
        let created = unsafe {
            self.raw()
                .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
        };
        let raw = match created {
            Ok(pipelines) => pipelines[0],
            Err((_, e)) => {
                // SAFETY: the layout is unused by any pipeline.
                unsafe { self.raw().destroy_pipeline_layout(layout, None) };
                return Err(e.into());
            }
        };
        self.set_name(raw, desc.name);
        Ok(Pipeline {
            device: Arc::clone(self),
            raw,
            layout,
            bind_point: vk::PipelineBindPoint::GRAPHICS,
            push_constant_stages: stages_mask,
        })
    }

    /// Creates a mesh-shading pipeline. Requires the mesh-shader feature.
    pub fn create_mesh_pipeline(self: &Arc<Self>, desc: &MeshPipelineDesc<'_>) -> Result<Pipeline> {
        let all_stages = vk::ShaderStageFlags::TASK_EXT
            | vk::ShaderStageFlags::MESH_EXT
            | vk::ShaderStageFlags::FRAGMENT;
        let layout = self.create_layout(all_stages, desc.push_constant_bytes)?;

        let names: Vec<CString> = [
            desc.task.map(|t| t.1),
            Some(desc.mesh.1),
            Some(desc.fragment.1),
        ]
        .into_iter()
        .flatten()
        .map(|n| CString::new(n).expect("entry point name"))
        .collect();
        let mut stages = Vec::new();
        let mut name_iter = names.iter();
        if let Some((module, _)) = desc.task {
            stages.push(
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::TASK_EXT)
                    .module(module)
                    .name(name_iter.next().expect("task name")),
            );
        }
        stages.push(
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::MESH_EXT)
                .module(desc.mesh.0)
                .name(name_iter.next().expect("mesh name")),
        );
        stages.push(
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(desc.fragment.0)
                .name(name_iter.next().expect("fragment name")),
        );

        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(if desc.wireframe {
                vk::PolygonMode::LINE
            } else {
                vk::PolygonMode::FILL
            })
            .cull_mode(desc.cull_mode)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(desc.depth_test)
            .depth_write_enable(desc.depth_test)
            .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL);
        let blend_attachments: Vec<_> = desc
            .color_formats
            .iter()
            .map(|_| {
                vk::PipelineColorBlendAttachmentState::default()
                    .color_write_mask(vk::ColorComponentFlags::RGBA)
            })
            .collect();
        let blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(desc.color_formats);
        if let Some(depth_format) = desc.depth_format {
            rendering = rendering.depth_attachment_format(depth_format);
        }
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .push_next(&mut rendering);
        // SAFETY: all referenced state lives until the call returns; no vertex input state is
        // required for mesh pipelines.
        let created = unsafe {
            self.raw()
                .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
        };
        let raw = match created {
            Ok(pipelines) => pipelines[0],
            Err((_, e)) => {
                // SAFETY: the layout is unused by any pipeline.
                unsafe { self.raw().destroy_pipeline_layout(layout, None) };
                return Err(e.into());
            }
        };
        self.set_name(raw, desc.name);
        Ok(Pipeline {
            device: Arc::clone(self),
            raw,
            layout,
            bind_point: vk::PipelineBindPoint::GRAPHICS,
            push_constant_stages: all_stages,
        })
    }
}

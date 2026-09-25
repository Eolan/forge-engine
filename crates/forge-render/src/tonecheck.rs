//! The ACES 2.0 check (issue #76, `shaders/tonecheck.slang`): the GPU runs both ACES 2.0
//! curves, the baked table and the per-pixel transform, over 4096 fixed scene colours, and
//! the CPU compares them with its own port ([`crate::aces2`], checked against OpenColorIO's
//! test values). `meshlets --tone-check` runs it; it passes when the GPU's per-pixel
//! transform is within one 8-bit code of the CPU's, and its table within one code of the
//! CPU's reading of the same table. The table's own error against the transform is reported
//! beside.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, GraphBuffer,
    MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};

use crate::aces2;
use crate::display::{ToneTables, ToneTablesPush};

/// Scene colours checked.
const COLOURS: usize = 4096;

/// Mirrors `CheckPush` in `tonecheck.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CheckPush {
    input: u64,
    output: u64,
    count: u32,
    pad: u32,
    tables: ToneTablesPush,
}

/// The largest differences found, in 8-bit sRGB codes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneCheckResult {
    /// The GPU's per-pixel transform against the CPU's.
    pub analytic: f32,
    /// The GPU's table against the CPU reading the same table.
    pub table: f32,
    /// The table against the transform (both on the GPU): p50, p99, p99.9 and max.
    pub table_error: [f32; 4],
    /// Colours compared.
    pub colours: usize,
}

impl ToneCheckResult {
    /// Whether both GPU paths agree with the CPU within one code.
    pub fn passes(&self) -> bool {
        self.colours > 0 && self.analytic < 1.0 && self.table < 1.0
    }
}

/// The check's pipeline, its colours and its result buffer.
pub struct ToneCheck {
    pipeline: Pipeline,
    tables: ToneTables,
    input: Buffer,
    /// Per colour: the table's display colour, then the per-pixel one (float4 each).
    output: GraphBuffer,
    colours: Vec<[f32; 3]>,
}

impl ToneCheck {
    /// Compiles the pass and uploads the colours.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("tonecheck.slang", "check_main", ShaderStage::Compute)?,
            "tone check",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "check_main"),
            push_constant_bytes: std::mem::size_of::<CheckPush>() as u32,
            name: "tone check",
        });
        device.destroy_shader_module(module);
        let colours = aces2::test_colours(COLOURS);
        let padded: Vec<[f32; 4]> = colours.iter().map(|c| [c[0], c[1], c[2], 0.0]).collect();
        let input = device.create_buffer_with_data(
            &padded,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Transfer,
            "tone check colours",
        )?;
        let output = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: (COLOURS * 2 * 16) as u64,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name: "tone check result",
        })?);
        Ok(Self {
            pipeline: pipeline?,
            tables: ToneTables::new(device)?,
            input,
            output,
            colours,
        })
    }

    /// Declares the pass "check/tone" (4096 colours, both paths).
    pub fn record<'f>(&'f self, graph: &mut FrameGraph<'f>) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let output = graph.import_buffer(&self.output);
        let pipeline = &self.pipeline;
        let push = CheckPush {
            input: self.input.address(),
            output: self.output.address(),
            count: COLOURS as u32,
            pad: 0,
            tables: self.tables.push(),
        };
        graph
            .pass("check/tone")
            .buffer(output, BufferAccess::ShaderWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &push);
                commands.dispatch((COLOURS as u32).div_ceil(64), 1, 1);
                Ok(())
            });
        graph
            .pass("check/tone")
            .buffer(output, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// The last completed check (call once the device is idle).
    pub fn result(&self) -> ToneCheckResult {
        let mut out = vec![[0.0_f32; 4]; COLOURS * 2];
        self.output.read(0, &mut out);
        let sdr = aces2::sdr();
        let table = aces2::bake(sdr, aces2::LUT_SIZE);
        let rgb = |v: [f32; 4]| [v[0], v[1], v[2]];
        let (mut analytic, mut table_max) = (0.0_f32, 0.0_f32);
        let mut errors = Vec::with_capacity(COLOURS);
        for (i, &colour) in self.colours.iter().enumerate() {
            let (gpu_table, gpu_analytic) = (rgb(out[2 * i]), rgb(out[2 * i + 1]));
            analytic = analytic.max(aces2::code_difference(gpu_analytic, sdr.apply(colour)));
            let cpu_table = aces2::sample(&table, aces2::LUT_SIZE, colour);
            table_max = table_max.max(aces2::code_difference(gpu_table, cpu_table));
            errors.push(aces2::code_difference(gpu_table, gpu_analytic));
        }
        errors.sort_unstable_by(f32::total_cmp);
        let at = |p: f64| errors[((p * errors.len() as f64) as usize).min(errors.len() - 1)];
        ToneCheckResult {
            analytic,
            table: table_max,
            table_error: [at(0.5), at(0.99), at(0.999), errors[errors.len() - 1]],
            colours: self.colours.len(),
        }
    }
}

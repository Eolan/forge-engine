//! The ACES 2.0 check (issues #76, #94; `shaders/tonecheck.slang`): the GPU runs both ACES 2.0
//! curves, the baked table and the per-pixel transform, for the SDR display and through the
//! 1000-nit HDR preset, over 4096 fixed scene colours, and the CPU compares them with its own
//! port ([`crate::aces2`], checked against OpenColorIO's test values). `meshlets --tone-check`
//! runs it; it passes when the GPU's per-pixel transforms are within one code of the CPU's
//! (8-bit sRGB for SDR, 10-bit PQ for HDR), and its tables within one code of the CPU's
//! reading of the same tables. The tables' own error against the transform is reported beside.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, GraphBuffer,
    MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};

use crate::aces2::{self, Preset};
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

/// The largest differences found: in 8-bit sRGB codes for SDR, 10-bit PQ codes for HDR.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneCheckResult {
    /// The GPU's per-pixel transform against the CPU's.
    pub analytic: f32,
    /// The GPU's table against the CPU reading the same table.
    pub table: f32,
    /// The table against the transform (both on the GPU): p50, p99, p99.9 and max.
    pub table_error: [f32; 4],
    /// The HDR preset's per-pixel transform against the CPU's.
    pub hdr_analytic: f32,
    /// The HDR preset's table against the CPU reading the same table.
    pub hdr_table: f32,
    /// The HDR table against the HDR transform (both on the GPU): p50, p99, p99.9 and max.
    pub hdr_table_error: [f32; 4],
    /// Colours compared.
    pub colours: usize,
}

impl ToneCheckResult {
    /// Whether every GPU path agrees with the CPU within one code.
    pub fn passes(&self) -> bool {
        self.colours > 0
            && self.analytic < 1.0
            && self.table < 1.0
            && self.hdr_analytic < 1.0
            && self.hdr_table < 1.0
    }
}

/// The check's pipeline, its colours and its result buffer.
pub struct ToneCheck {
    pipeline: Pipeline,
    tables: ToneTables,
    input: Buffer,
    /// Per colour: the SDR table's display colour, the SDR per-pixel one, then the HDR
    /// preset's two PQ signals (float4 each).
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
            size: (COLOURS * 4 * 16) as u64,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name: "tone check result",
        })?);
        let mut tables = ToneTables::new(device)?;
        tables.set_hdr(Preset::default())?;
        Ok(Self {
            pipeline: pipeline?,
            tables,
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
        let mut out = vec![[0.0_f32; 4]; COLOURS * 4];
        self.output.read(0, &mut out);
        let sdr = aces2::sdr();
        let table = aces2::bake(sdr, aces2::LUT_SIZE);
        let hdr = aces2::Hdr::new(Preset::default());
        let hdr_table = aces2::bake_hdr(&hdr, aces2::LUT_SIZE);
        let rgb = |v: [f32; 4]| [v[0], v[1], v[2]];
        let (mut analytic, mut table_max) = (0.0_f32, 0.0_f32);
        let (mut hdr_analytic, mut hdr_table_max) = (0.0_f32, 0.0_f32);
        let mut errors = Vec::with_capacity(COLOURS);
        let mut hdr_errors = Vec::with_capacity(COLOURS);
        for (i, &colour) in self.colours.iter().enumerate() {
            let (gpu_table, gpu_analytic) = (rgb(out[4 * i]), rgb(out[4 * i + 1]));
            analytic = analytic.max(aces2::code_difference(gpu_analytic, sdr.apply(colour)));
            let cpu_table = aces2::sample(&table, aces2::LUT_SIZE, colour);
            table_max = table_max.max(aces2::code_difference(gpu_table, cpu_table));
            errors.push(aces2::code_difference(gpu_table, gpu_analytic));
            // The HDR preset, in 10-bit PQ codes.
            let (gpu_table, gpu_analytic) = (rgb(out[4 * i + 2]), rgb(out[4 * i + 3]));
            hdr_analytic = hdr_analytic.max(aces2::pq_code_difference(
                gpu_analytic,
                hdr.apply_pq(colour),
            ));
            let cpu_table = aces2::sample_hdr(&hdr_table, aces2::LUT_SIZE, hdr.lut_max(), colour);
            hdr_table_max = hdr_table_max.max(aces2::pq_code_difference(gpu_table, cpu_table));
            hdr_errors.push(aces2::pq_code_difference(gpu_table, gpu_analytic));
        }
        let spread = |errors: &mut Vec<f32>| {
            errors.sort_unstable_by(f32::total_cmp);
            let at = |p: f64| errors[((p * errors.len() as f64) as usize).min(errors.len() - 1)];
            [at(0.5), at(0.99), at(0.999), errors[errors.len() - 1]]
        };
        ToneCheckResult {
            analytic,
            table: table_max,
            table_error: spread(&mut errors),
            hdr_analytic,
            hdr_table: hdr_table_max,
            hdr_table_error: spread(&mut hdr_errors),
            colours: self.colours.len(),
        }
    }
}

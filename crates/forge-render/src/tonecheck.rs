//! The ACES 2.0 check (issues #76, #94; `shaders/tonecheck.slang`): the GPU runs both ACES 2.0
//! curves, the baked table and the per-pixel transform, for the SDR display and through the
//! 1000-nit HDR preset, over 4096 fixed scene colours, and the CPU compares them with its own
//! port ([`crate::aces2`], checked against OpenColorIO's test values). `meshlets --tone-check`
//! runs it; it passes when the GPU's per-pixel transforms are within one code of the CPU's
//! (8-bit sRGB for SDR, 10-bit PQ for HDR), and its tables within one code of the CPU's
//! reading of the same tables. The tables' own error against the transform is reported beside.
//! A scene's grading table (D-100) is checked the same way: a fixed test table through
//! `TONEMAP_SCENE_LUT`, against [`Grading::apply`], within one 8-bit code.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FrameGraph, GraphBuffer,
    MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};

use crate::aces2::{self, Preset};
use crate::display::{Grading, GradingEncoding, ToneTables, ToneTablesPush};

/// Scene colours checked.
const COLOURS: usize = 4096;
/// Results per colour: ACES 2.0's SDR table and transform, its HDR table and transform, a scene's
/// grading table.
const OUTPUTS: usize = 5;

/// The scene grading table the check runs (17³ over LogC v3's grid): a warm grade, its red
/// lifted, its blue lowered, so a swapped axis or a slice off by one shows. Built once.
pub fn test_grading() -> &'static Grading {
    static TABLE: std::sync::OnceLock<Grading> = std::sync::OnceLock::new();
    TABLE.get_or_init(build_test_grading)
}

fn build_test_grading() -> Grading {
    let n = 17u32;
    let e = GradingEncoding::LogC3;
    let grid = |i: usize| e.decode(i as f32 / (n - 1) as f32).clamp(0.0, 1.0);
    let mut texels = Vec::with_capacity((n as usize).pow(3));
    for b in 0..n as usize {
        for g in 0..n as usize {
            for r in 0..n as usize {
                texels.push([grid(r).powf(0.8), grid(g) * 0.95, grid(b) * 0.7]);
            }
        }
    }
    Grading {
        size: n,
        texels,
        encoding: e,
    }
}

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
    /// A scene's grading table (D-100, [`test_grading`]) on the GPU against [`Grading::apply`].
    pub scene: f32,
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
            && self.scene < 1.0
    }
}

/// The check's pipeline, its colours and its result buffer.
pub struct ToneCheck {
    pipeline: Pipeline,
    tables: ToneTables,
    input: Buffer,
    /// Per colour: the SDR table's display colour, the SDR per-pixel one, then the HDR
    /// preset's two PQ signals, then the scene table's display colour (float4 each, `OUTPUTS`).
    output: GraphBuffer,
    colours: Vec<[f32; 3]>,
    /// The scene grading table on the GPU ([`test_grading`]), for the CPU's reading.
    grading: &'static Grading,
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
            size: (COLOURS * OUTPUTS * 16) as u64,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuToCpu,
            category: MemoryCategory::Transfer,
            name: "tone check result",
        })?);
        let mut tables = ToneTables::new(device)?;
        tables.set_hdr(Preset::default())?;
        let grading = test_grading();
        tables.set_grading(Some(grading))?;
        Ok(Self {
            pipeline: pipeline?,
            tables,
            grading,
            input,
            output,
            colours,
        })
    }

    /// Declares the pass "check/tone" (4096 colours: ACES 2.0's paths and the scene table).
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
        let mut out = vec![[0.0_f32; 4]; COLOURS * OUTPUTS];
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
        let mut scene = 0.0_f32;
        for (i, &colour) in self.colours.iter().enumerate() {
            let (gpu_table, gpu_analytic) = (rgb(out[OUTPUTS * i]), rgb(out[OUTPUTS * i + 1]));
            analytic = analytic.max(aces2::code_difference(gpu_analytic, sdr.apply(colour)));
            let cpu_table = aces2::sample(&table, aces2::LUT_SIZE, colour);
            table_max = table_max.max(aces2::code_difference(gpu_table, cpu_table));
            errors.push(aces2::code_difference(gpu_table, gpu_analytic));
            // The HDR preset, in 10-bit PQ codes.
            let (gpu_table, gpu_analytic) = (rgb(out[OUTPUTS * i + 2]), rgb(out[OUTPUTS * i + 3]));
            hdr_analytic = hdr_analytic.max(aces2::pq_code_difference(
                gpu_analytic,
                hdr.apply_pq(colour),
            ));
            let cpu_table = aces2::sample_hdr(&hdr_table, aces2::LUT_SIZE, hdr.lut_max(), colour);
            hdr_table_max = hdr_table_max.max(aces2::pq_code_difference(gpu_table, cpu_table));
            hdr_errors.push(aces2::pq_code_difference(gpu_table, gpu_analytic));
            // The scene's grading table, in 8-bit sRGB codes.
            let cpu_scene = self.grading.apply(glam::Vec3::from(colour)).to_array();
            scene = scene.max(aces2::code_difference(rgb(out[OUTPUTS * i + 4]), cpu_scene));
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
            scene,
            colours: self.colours.len(),
        }
    }
}

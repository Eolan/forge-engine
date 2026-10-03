//! The display transform: a pre-exposed, scene-referred HDR colour in, a display colour out
//! (`shaders/tonemap.slang`, `shaders/display.slang`).
//!
//! The tone curve is data chosen at run time ([`Tonemap`]), never baked into lighting: AgX
//! is the engine default (hue-safe), the ACES fit the contrasty film look (the ballad uses
//! it: its toe keeps space black), Khronos PBR Neutral the view that keeps base colours for
//! material checks, ACES 2.0 the Academy's current output transform ([`crate::aces2`]). The
//! ballad applies it inside its TAA resolve (one pass writes the HDR history and the display
//! image); [`Display`] is the stand-alone pass for paths without temporal filtering. The CPU
//! functions below mirror the shader and pin its behaviour in tests.

use std::cell::Cell;
use std::str::FromStr;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, Device, FrameGraph, FullscreenPipelineDesc, Image, ImageAccess, ImageDesc, ImageHandle,
    MemoryCategory, Pipeline, Result, SampledImageId, ShaderCompiler, ShaderStage, vk,
};
use glam::Vec3;

use crate::aces2::{self, Preset};

/// A tone curve (index as in `tonemap.slang`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemap {
    /// AgX (Sobotka; Blender 4.0's default): hue-safe, bright colours desaturate to white.
    #[default]
    AgX,
    /// AgX with its "punchy" look (Wrensch 2023, as Filament's): deeper blacks, more saturated
    /// colours (D-045).
    AgXPunchy,
    /// ACES 1.x RRT + sRGB ODT, Hill's fit: contrasty, film-like, darker mid-tones.
    Aces,
    /// Khronos PBR Neutral: base colours unchanged up to ~0.76, highlights compressed.
    PbrNeutral,
    /// ACES 2.0's output transform for a 100-nit SDR Rec.709 display, through its baked
    /// 65³ table: fewer hue skews than ACES 1, bright saturated colours go to white.
    Aces2,
    /// The same transform evaluated per pixel: the reference the table is measured against
    /// (not in the cycle; `--tonemap aces2-analytic`).
    Aces2Analytic,
}

impl Tonemap {
    /// Every curve, in cycling order.
    pub const ALL: [Tonemap; 5] = [
        Tonemap::AgX,
        Tonemap::AgXPunchy,
        Tonemap::Aces,
        Tonemap::PbrNeutral,
        Tonemap::Aces2,
    ];

    /// The shader's index.
    pub fn index(self) -> u32 {
        match self {
            Tonemap::AgX => 0,
            Tonemap::Aces => 1,
            Tonemap::PbrNeutral => 2,
            Tonemap::Aces2 => 3,
            Tonemap::Aces2Analytic => 4,
            Tonemap::AgXPunchy => 5,
        }
    }

    /// Short name for overlays and file names.
    pub fn name(self) -> &'static str {
        match self {
            Tonemap::AgX => "agx",
            Tonemap::AgXPunchy => "agx-punchy",
            Tonemap::Aces => "aces",
            Tonemap::PbrNeutral => "neutral",
            Tonemap::Aces2 => "aces2",
            Tonemap::Aces2Analytic => "aces2-analytic",
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Tonemap::AgX => "AgX",
            Tonemap::AgXPunchy => "AgX punchy",
            Tonemap::Aces => "ACES (Hill fit)",
            Tonemap::PbrNeutral => "Khronos PBR Neutral",
            Tonemap::Aces2 => "ACES 2.0 (SDR, table)",
            Tonemap::Aces2Analytic => "ACES 2.0 (SDR, per pixel)",
        }
    }

    /// The next curve (a key cycles through them).
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    /// CPU mirror of `tonemap()` in the shader: linear display colour in [0, 1] (before the
    /// sRGB encoding). Both ACES 2.0 curves give the transform itself; the table's error
    /// against it is measured in [`crate::aces2`].
    pub fn apply(self, color: Vec3) -> Vec3 {
        let display = match self {
            Tonemap::AgX => agx(color, false),
            Tonemap::AgXPunchy => agx(color, true),
            Tonemap::Aces => aces(color),
            Tonemap::PbrNeutral => pbr_neutral(color),
            Tonemap::Aces2 | Tonemap::Aces2Analytic => {
                Vec3::from(aces2::sdr().apply(color.max(Vec3::ZERO).to_array()))
            }
        };
        display.clamp(Vec3::ZERO, Vec3::ONE)
    }
}

impl FromStr for Tonemap {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .chain([Tonemap::Aces2Analytic])
            .find(|t| t.name().eq_ignore_ascii_case(text))
            .ok_or_else(|| {
                format!(
                    "unknown tone curve {text:?}: agx, agx-punchy, aces, neutral, aces2 or aces2-analytic"
                )
            })
    }
}

/// What the ACES 2.0 curves read on the GPU (issue #76): the baked table, a bindless
/// texture, and the per-pixel transform's parameters and tables in a buffer. Both are
/// written once, here. An HDR output adds its preset's table and parameters (issue #94),
/// baked when first asked for and again when the preset changes.
pub struct ToneTables {
    device: Arc<Device>,
    _lut: Image,
    lut: SampledImageId,
    params: Buffer,
    hdr: Option<HdrTables>,
}

/// One HDR preset's table and parameters.
struct HdrTables {
    preset: Preset,
    _lut: Image,
    lut: SampledImageId,
    params: Buffer,
    lut_max: f32,
}

/// Mirrors `ToneTables` in `tonemap.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ToneTablesPush {
    lut: u32,
    hdr_lut: u32,
    aces2: u64,
    hdr: u64,
    hdr_max: f32,
    pad: u32,
}

impl ToneTables {
    /// Uploads the table (baked on first use, about 10 ms) and the parameters.
    pub fn new(device: &Arc<Device>) -> Result<Self> {
        let size = aces2::LUT_SIZE;
        let lut = device.create_image_with_data(
            ImageDesc {
                width: size * size,
                height: size,
                format: vk::Format::R16G16B16A16_SFLOAT,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "ACES 2.0 table",
            },
            aces2::shared_lut_texels(),
        )?;
        let sampled =
            device.register_sampled_image(lut.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let params = device.create_buffer_with_data(
            &aces2::sdr().gpu_params(),
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Textures,
            "ACES 2.0 parameters",
        )?;
        Ok(Self {
            device: Arc::clone(device),
            _lut: lut,
            lut: sampled,
            params,
            hdr: None,
        })
    }

    /// Uploads `preset`'s table (baked on its first use in the process, 10–15 ms) and
    /// parameters unless they are already there. Waits for the device first when it replaces
    /// another preset's.
    pub fn set_hdr(&mut self, preset: Preset) -> Result<()> {
        if self.hdr.as_ref().is_some_and(|h| h.preset == preset) {
            return Ok(());
        }
        let (texels, lut_max) = aces2::shared_hdr_lut_texels(preset);
        let size = aces2::LUT_SIZE;
        let lut = self.device.create_image_with_data(
            ImageDesc {
                width: size * size,
                height: size,
                format: vk::Format::R16G16B16A16_UNORM,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "ACES 2.0 HDR table",
            },
            &texels,
        )?;
        let sampled = self
            .device
            .register_sampled_image(lut.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let params = self.device.create_buffer_with_data(
            &aces2::Hdr::new(preset).gpu_params(),
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Textures,
            "ACES 2.0 HDR parameters",
        )?;
        if let Some(old) = self.hdr.take() {
            self.device.wait_idle();
            self.device.release_sampled_image(old.lut);
        }
        self.hdr = Some(HdrTables {
            preset,
            _lut: lut,
            lut: sampled,
            params,
            lut_max,
        });
        Ok(())
    }

    /// The fields of a push constant block. Without an HDR preset the HDR fields repeat the
    /// SDR ones (only HDR outputs read them).
    pub fn push(&self) -> ToneTablesPush {
        let (hdr_lut, hdr, hdr_max) = match &self.hdr {
            Some(h) => (h.lut.0, h.params.address(), h.lut_max),
            None => (self.lut.0, self.params.address(), aces2::LUT_MAX),
        };
        ToneTablesPush {
            lut: self.lut.0,
            hdr_lut,
            aces2: self.params.address(),
            hdr,
            hdr_max,
            pad: 0,
        }
    }
}

impl Drop for ToneTables {
    fn drop(&mut self) {
        self.device.release_sampled_image(self.lut);
        if let Some(hdr) = self.hdr.take() {
            self.device.release_sampled_image(hdr.lut);
        }
    }
}

/// How the display image is encoded (`OUTPUT_*` in `tonemap.slang`, issue #94).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEncoding {
    /// An sRGB target: it encodes on write.
    SrgbTarget,
    /// A UNORM SDR target: the shader encodes sRGB.
    Srgb,
    /// HDR10: Rec.2100 PQ over Rec.2020 primaries in a 10-bit target.
    Pq,
    /// scRGB: linear Rec.709 in half floats, 1.0 being 80 nits.
    ScRgb,
}

impl OutputEncoding {
    /// The encoding a target of `format` gets: Forge creates HDR10 targets as 10-bit UNORM
    /// and scRGB ones as half floats, and SDR ones as 8-bit sRGB or UNORM.
    pub fn for_format(format: vk::Format) -> Self {
        match format {
            vk::Format::A2B10G10R10_UNORM_PACK32 | vk::Format::A2R10G10B10_UNORM_PACK32 => {
                OutputEncoding::Pq
            }
            vk::Format::R16G16B16A16_SFLOAT => OutputEncoding::ScRgb,
            f if format_encodes_srgb(f) => OutputEncoding::SrgbTarget,
            _ => OutputEncoding::Srgb,
        }
    }

    /// The shader's index.
    pub fn index(self) -> u32 {
        match self {
            OutputEncoding::SrgbTarget => 0,
            OutputEncoding::Srgb => 1,
            OutputEncoding::Pq => 2,
            OutputEncoding::ScRgb => 3,
        }
    }

    /// Whether the output is HDR.
    pub fn is_hdr(self) -> bool {
        matches!(self, OutputEncoding::Pq | OutputEncoding::ScRgb)
    }
}

/// An HDR output's settings (issue #94, D-022): ACES 2.0's preset, the paper-white offset and
/// the UI's white. SDR outputs ignore them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrOutput {
    /// ACES 2.0's preset (the display's peak, the limiting gamut).
    pub preset: Preset,
    /// The paper-white offset: stops added to the scene before ACES 2.0. 0, the default, is
    /// the Academy's look (a scene white at 107 nits at 1000 nits; the owner's pick).
    pub scene_stops: f32,
    /// The nits of the UI's white, and of the other curves' white (their SDR image): the
    /// OS's SDR white level when known, BT.2408's 203 nits otherwise.
    pub sdr_white: f32,
}

impl HdrOutput {
    /// The settings for a display of `peak` nits (ACES 2.0's preset not above it, P3-D65
    /// limited), with `scene_stops` of paper-white offset and the UI's white at `sdr_white`.
    pub fn new(peak: f32, scene_stops: f32, sdr_white: f32) -> Self {
        Self {
            preset: Preset::for_display(peak),
            scene_stops,
            sdr_white,
        }
    }
}

impl Default for HdrOutput {
    fn default() -> Self {
        Self {
            preset: Preset::default(),
            scene_stops: 0.0,
            sdr_white: 203.0,
        }
    }
}

/// Mirrors `Output` in `tonemap.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct OutputPush {
    encoding: u32,
    scene_gain: f32,
    sdr_white: f32,
    frame: u32,
}

impl OutputPush {
    /// The block for `encoding` with `hdr`'s settings, dithered for `frame`.
    pub fn new(encoding: OutputEncoding, hdr: &HdrOutput, frame: u32) -> Self {
        Self {
            encoding: encoding.index(),
            scene_gain: hdr.scene_stops.exp2(),
            sdr_white: hdr.sdr_white,
            frame,
        }
    }
}

/// Rec.709 to Rec.2020, both D65 (BT.2087), as `tonemap.slang` writes it; rows.
pub const REC709_TO_REC2020: [[f32; 3]; 3] = [
    [0.627_403_9, 0.329_283_04, 0.043_313_07],
    [0.069_097_29, 0.919_540_4, 0.011_362_32],
    [0.016_391_44, 0.088_013_31, 0.895_595_25],
];

/// The CPU mirror of `display_output`'s HDR side, before the dither: the PQ signal (or for
/// scRGB, the linear value) of a pre-exposed colour through `curve`.
pub fn hdr_output(color: Vec3, curve: Tonemap, encoding: OutputEncoding, hdr: &HdrOutput) -> Vec3 {
    let signal = match curve {
        Tonemap::Aces2 | Tonemap::Aces2Analytic => Vec3::from(
            aces2::Hdr::new(hdr.preset)
                .apply_pq((color * hdr.scene_stops.exp2()).max(Vec3::ZERO).to_array()),
        ),
        _ => {
            let display = curve.apply(color);
            let rec2020 = rows(REC709_TO_REC2020, display) * hdr.sdr_white;
            Vec3::from(rec2020.to_array().map(aces2::pq_encode))
        }
    };
    match encoding {
        OutputEncoding::ScRgb => {
            let nits = Vec3::from(signal.to_array().map(aces2::pq_decode));
            rows(REC2020_TO_REC709, nits) / 80.0
        }
        _ => signal,
    }
}

/// Rec.2020 to Rec.709 (the inverse of [`REC709_TO_REC2020`]), as `tonemap.slang` writes it.
pub const REC2020_TO_REC709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_86],
    [-0.124_550_47, 1.132_899_9, -0.008_349_42],
    [-0.018_150_76, -0.100_578_9, 1.118_729_7],
];

/// Whether a target of `format` encodes sRGB on write (so the shader must not).
pub fn format_encodes_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB | vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

fn rows(m: [[f32; 3]; 3], v: Vec3) -> Vec3 {
    Vec3::new(
        Vec3::from(m[0]).dot(v),
        Vec3::from(m[1]).dot(v),
        Vec3::from(m[2]).dot(v),
    )
}

fn agx(color: Vec3, punchy: bool) -> Vec3 {
    const MIN_EV: f32 = -12.47393;
    const MAX_EV: f32 = 4.026069;
    let inset = [
        [0.842_479_06, 0.078_433_6, 0.079_223_745],
        [0.042_328_242, 0.878_468_6, 0.079_166_13],
        [0.042_375_655, 0.078_433_6, 0.879_143],
    ];
    let outset = [
        [1.196_879, -0.098_020_88, -0.099_029_74],
        [-0.052_896_85, 1.151_903_1, -0.098_961_18],
        [-0.052_971_635, -0.098_043_45, 1.151_073_7],
    ];
    let v = rows(inset, color.max(Vec3::ZERO));
    let v = v
        .max(Vec3::splat(1e-10))
        .map(f32::log2)
        .clamp(Vec3::splat(MIN_EV), Vec3::splat(MAX_EV));
    let x = (v - MIN_EV) / (MAX_EV - MIN_EV);
    let x2 = x * x;
    let x4 = x2 * x2;
    let curve =
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
            - Vec3::splat(0.00232);
    let curve = if punchy {
        let luma = curve.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        Vec3::splat(luma) + 1.4 * (curve.max(Vec3::ZERO).powf(1.35) - Vec3::splat(luma))
    } else {
        curve
    };
    rows(outset, curve).max(Vec3::ZERO).powf(2.2)
}

fn aces(color: Vec3) -> Vec3 {
    let input = [
        [0.59719, 0.35458, 0.04823],
        [0.07600, 0.90834, 0.01566],
        [0.02840, 0.13383, 0.83777],
    ];
    let output = [
        [1.60475, -0.53108, -0.07367],
        [-0.10208, 1.10813, -0.00605],
        [-0.00327, -0.07276, 1.07602],
    ];
    let v = rows(input, color.max(Vec3::ZERO));
    let a = v * (v + 0.024_578_6) - Vec3::splat(0.000_090_537);
    let b = v * (0.983_729 * v + 0.432_951) + Vec3::splat(0.238_081);
    rows(output, a / b)
}

fn pbr_neutral(color: Vec3) -> Vec3 {
    const START_COMPRESSION: f32 = 0.8 - 0.04;
    const DESATURATION: f32 = 0.15;
    let mut c = color.max(Vec3::ZERO);
    let lowest = c.min_element();
    let offset = if lowest < 0.08 {
        lowest - 6.25 * lowest * lowest
    } else {
        0.04
    };
    c -= Vec3::splat(offset);
    let peak = c.max_element();
    if peak < START_COMPRESSION {
        return c;
    }
    let d = 1.0 - START_COMPRESSION;
    let new_peak = 1.0 - d * d / (peak + d - START_COMPRESSION);
    c *= new_peak / peak;
    let g = 1.0 - 1.0 / (DESATURATION * (peak - new_peak) + 1.0);
    c.lerp(Vec3::splat(new_peak), g)
}

/// Mirrors `DisplayPush` in `display.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DisplayPush {
    image: u32,
    curve: u32,
    output: OutputPush,
    tables: ToneTablesPush,
}

/// The stand-alone display pass.
pub struct Display {
    device: Arc<Device>,
    pipeline: Pipeline,
    format: vk::Format,
    hdr: HdrOutput,
    frame: Cell<u32>,
    tables: ToneTables,
}

impl Display {
    /// Compiles the pass for an output image of `output_format`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        output_format: vk::Format,
    ) -> Result<Self> {
        let mut tables = ToneTables::new(device)?;
        let hdr = HdrOutput::default();
        if OutputEncoding::for_format(output_format).is_hdr() {
            tables.set_hdr(hdr.preset)?;
        }
        Ok(Self {
            device: Arc::clone(device),
            pipeline: Self::pipeline(device, shaders, output_format)?,
            format: output_format,
            hdr,
            frame: Cell::new(0),
            tables,
        })
    }

    /// Follows the output (issue #94): a new target format recompiles the pass (call it while
    /// no frame uses the old one, as after a swapchain's recreation), an HDR one bakes its
    /// preset's table.
    pub fn set_output(
        &mut self,
        shaders: &ShaderCompiler,
        format: vk::Format,
        hdr: HdrOutput,
    ) -> Result<()> {
        if format != self.format {
            self.pipeline = Self::pipeline(&self.device, shaders, format)?;
            self.format = format;
        }
        if OutputEncoding::for_format(format).is_hdr() {
            self.tables.set_hdr(hdr.preset)?;
        }
        self.hdr = hdr;
        Ok(())
    }

    fn pipeline(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        output_format: vk::Format,
    ) -> Result<Pipeline> {
        let vertex = device.create_shader_module(
            &shaders.compile("display.slang", "vert_main", ShaderStage::Vertex)?,
            "display vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("display.slang", "frag_main", ShaderStage::Fragment)?,
            "display fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[output_format],
            push_constant_bytes: std::mem::size_of::<DisplayPush>() as u32,
            alpha_blend: false,
            depth_test: None,
            depth_write: false,
            name: "display transform",
        })?;
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        Ok(pipeline)
    }

    /// Declares the pass "post/display transform": `src` (pre-exposed HDR) through `curve`
    /// into every pixel of `dst`.
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        src: ImageHandle,
        dst: ImageHandle,
        extent: vk::Extent2D,
        curve: Tonemap,
    ) {
        let pipeline = &self.pipeline;
        let frame = self.frame.get();
        self.frame.set(frame.wrapping_add(1));
        let output = OutputPush::new(OutputEncoding::for_format(self.format), &self.hdr, frame);
        let tables = self.tables.push();
        graph
            .pass("post/display transform")
            .image(
                src,
                ImageAccess::Sampled(vk::PipelineStageFlags2::FRAGMENT_SHADER),
            )
            .image(dst, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let attachments = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(dst))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::DONT_CARE)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&attachments);
                commands.begin_rendering(&info);
                commands.bind_pipeline(pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(
                    pipeline,
                    &DisplayPush {
                        image: resources.sampled(src).0,
                        curve: curve.index(),
                        output,
                        tables,
                    },
                );
                commands.draw(3, 1);
                commands.end_rendering();
                Ok(())
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_stays_black_and_white_saturates() {
        for curve in Tonemap::ALL {
            let black = curve.apply(Vec3::ZERO);
            assert!(black.max_element() < 2e-3, "{curve:?} black {black}");
            let bright = curve.apply(Vec3::splat(1.0e4));
            assert!(bright.min_element() > 0.99, "{curve:?} white {bright}");
        }
    }

    #[test]
    fn grey_ramps_are_monotonic_and_stay_grey() {
        for curve in Tonemap::ALL {
            let mut last = -1.0;
            for i in 0..200 {
                let x = (i as f32 * 0.1 - 12.0).exp2();
                let y = curve.apply(Vec3::splat(x));
                assert!(y.x >= last - 1e-6, "{curve:?} not monotonic at {x}");
                assert!(
                    y.max_element() - y.min_element() < 2e-3,
                    "{curve:?} tints grey {y}"
                );
                last = y.x;
            }
        }
    }

    #[test]
    fn middle_grey_lands_where_each_curve_puts_it() {
        let grey = |curve: Tonemap| curve.apply(Vec3::splat(0.18)).x;
        // AgX lifts the mid-tones a little, ACES darkens them, Neutral keeps them (less its
        // 0.04 black offset).
        assert!(
            (0.19..0.24).contains(&grey(Tonemap::AgX)),
            "{}",
            grey(Tonemap::AgX)
        );
        assert!(
            (0.095..0.115).contains(&grey(Tonemap::Aces)),
            "{}",
            grey(Tonemap::Aces)
        );
        assert!((grey(Tonemap::PbrNeutral) - 0.14).abs() < 1e-4);
        // Neutral is the identity (plus offset) below its compression start.
        let base = Vec3::new(0.5, 0.3, 0.2);
        let out = Tonemap::PbrNeutral.apply(base);
        assert!((out - (base - Vec3::splat(0.04))).length() < 1e-5, "{out}");
    }

    #[test]
    fn agx_turns_a_blinding_red_towards_white() {
        let red = Tonemap::AgX.apply(Vec3::new(200.0, 1.0, 1.0));
        assert!(red.x > 0.95 && red.y > 0.5 && red.z > 0.5, "{red}");
        // A dim red stays red.
        let dim = Tonemap::AgX.apply(Vec3::new(0.2, 0.01, 0.01));
        assert!(dim.x > 3.0 * dim.y, "{dim}");
    }

    #[test]
    fn agx_punchy_deepens_the_blacks_and_keeps_the_white() {
        // The room's black squares in the sun (#159): AgX lifts them to a milky grey.
        let black = |curve: Tonemap| curve.apply(Vec3::splat(0.03)).x;
        assert!(
            black(Tonemap::AgXPunchy) < 0.3 * black(Tonemap::AgX),
            "{} against {}",
            black(Tonemap::AgXPunchy),
            black(Tonemap::AgX)
        );
        let white = Tonemap::AgXPunchy.apply(Vec3::splat(16.0)).x;
        assert!(white > 0.9, "{white}");
        // More saturated: a dim red's green falls further below its red.
        let red = |curve: Tonemap| {
            let c = curve.apply(Vec3::new(0.2, 0.05, 0.05));
            c.y / c.x
        };
        assert!(red(Tonemap::AgXPunchy) < red(Tonemap::AgX));
    }

    #[test]
    fn the_shader_s_gamut_matrices_match_the_primaries() {
        let check = |written: [[f32; 3]; 3], derived: [[f32; 3]; 3]| {
            for r in 0..3 {
                for c in 0..3 {
                    assert!(
                        (written[r][c] - derived[r][c]).abs() < 1e-6,
                        "{written:?} against {derived:?}"
                    );
                }
            }
        };
        check(
            REC709_TO_REC2020,
            aces2::conversion_rows(&aces2::REC709, &aces2::REC2020),
        );
        check(
            REC2020_TO_REC709,
            aces2::conversion_rows(&aces2::REC2020, &aces2::REC709),
        );
    }

    #[test]
    fn outputs_follow_the_target_format() {
        use OutputEncoding as E;
        assert_eq!(E::for_format(vk::Format::B8G8R8A8_SRGB), E::SrgbTarget);
        assert_eq!(E::for_format(vk::Format::B8G8R8A8_UNORM), E::Srgb);
        assert_eq!(E::for_format(vk::Format::A2B10G10R10_UNORM_PACK32), E::Pq);
        assert_eq!(E::for_format(vk::Format::R16G16B16A16_SFLOAT), E::ScRgb);
        assert!(E::Pq.is_hdr() && !E::Srgb.is_hdr());
        // The blocks the shaders mirror.
        assert_eq!(std::mem::size_of::<OutputPush>(), 16);
        assert_eq!(std::mem::size_of::<ToneTablesPush>(), 32);
        assert_eq!(std::mem::size_of::<DisplayPush>(), 56);
    }

    #[test]
    fn hdr_outputs_put_grey_and_the_ui_where_d_022_says() {
        let hdr = HdrOutput::default();
        let nits = |signal: Vec3| aces2::pq_decode(signal.y);
        // ACES 2.0 at 1000 nits, the Academy's look: grey at 14.5 nits.
        let grey = nits(hdr_output(
            Vec3::splat(0.18),
            Tonemap::Aces2,
            OutputEncoding::Pq,
            &hdr,
        ));
        assert!((14.0..15.2).contains(&grey), "{grey}");
        // One stop of paper-white offset lifts it 2.4 times (the curve's contrast steepens the
        // stop).
        let lifted = HdrOutput {
            scene_stops: 1.0,
            ..hdr
        };
        let brighter = nits(hdr_output(
            Vec3::splat(0.18),
            Tonemap::Aces2,
            OutputEncoding::Pq,
            &lifted,
        ));
        assert!(brighter > 2.0 * grey && brighter < 2.8 * grey, "{brighter}");
        // Another curve's white is the UI's.
        let white = nits(hdr_output(
            Vec3::splat(1.0e4),
            Tonemap::AgX,
            OutputEncoding::Pq,
            &hdr,
        ));
        assert!((white - 203.0).abs() < 2.0, "{white}"); // AgX tops out at 0.997
        // scRGB: the same light, 1.0 being 80 nits.
        let sc = hdr_output(
            Vec3::splat(1.0e4),
            Tonemap::AgX,
            OutputEncoding::ScRgb,
            &hdr,
        );
        assert!(
            (sc - Vec3::splat(203.0 / 80.0)).abs().max_element() < 0.03,
            "{sc}"
        );
    }

    #[test]
    fn curves_parse_and_cycle() {
        assert_eq!("AgX".parse::<Tonemap>(), Ok(Tonemap::AgX));
        assert_eq!("neutral".parse::<Tonemap>(), Ok(Tonemap::PbrNeutral));
        assert!("filmic".parse::<Tonemap>().is_err());
        let mut t = Tonemap::default();
        for _ in 0..Tonemap::ALL.len() {
            t = t.next();
        }
        assert_eq!(t, Tonemap::default());
        assert!(format_encodes_srgb(vk::Format::B8G8R8A8_SRGB));
        assert!(!format_encodes_srgb(vk::Format::B8G8R8A8_UNORM));
    }
}

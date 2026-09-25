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

use std::str::FromStr;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, Device, FrameGraph, FullscreenPipelineDesc, Image, ImageAccess, ImageDesc, ImageHandle,
    MemoryCategory, Pipeline, Result, SampledImageId, ShaderCompiler, ShaderStage, vk,
};
use glam::Vec3;

use crate::aces2;

/// A tone curve (index as in `tonemap.slang`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemap {
    /// AgX (Sobotka; Blender 4.0's default): hue-safe, bright colours desaturate to white.
    #[default]
    AgX,
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
    pub const ALL: [Tonemap; 4] = [
        Tonemap::AgX,
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
        }
    }

    /// Short name for overlays and file names.
    pub fn name(self) -> &'static str {
        match self {
            Tonemap::AgX => "agx",
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
            Tonemap::AgX => agx(color),
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
                format!("unknown tone curve {text:?}: agx, aces, neutral, aces2 or aces2-analytic")
            })
    }
}

/// What the ACES 2.0 curves read on the GPU (issue #76): the baked table, a bindless
/// texture, and the per-pixel transform's parameters and tables in a buffer. Both are
/// written once, here.
pub struct ToneTables {
    device: Arc<Device>,
    _lut: Image,
    lut: SampledImageId,
    params: Buffer,
}

/// Mirrors `ToneTables` in `tonemap.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ToneTablesPush {
    lut: u32,
    pad: u32,
    aces2: u64,
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
        })
    }

    /// The fields of a push constant block.
    pub fn push(&self) -> ToneTablesPush {
        ToneTablesPush {
            lut: self.lut.0,
            pad: 0,
            aces2: self.params.address(),
        }
    }
}

impl Drop for ToneTables {
    fn drop(&mut self) {
        self.device.release_sampled_image(self.lut);
    }
}

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

fn agx(color: Vec3) -> Vec3 {
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
    encode_srgb: u32,
    pad: u32,
    tables: ToneTablesPush,
}

/// The stand-alone display pass.
pub struct Display {
    pipeline: Pipeline,
    encode_srgb: bool,
    tables: ToneTables,
}

impl Display {
    /// Compiles the pass for an output image of `output_format`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        output_format: vk::Format,
    ) -> Result<Self> {
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
        Ok(Self {
            pipeline,
            encode_srgb: !format_encodes_srgb(output_format),
            tables: ToneTables::new(device)?,
        })
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
        let encode_srgb = u32::from(self.encode_srgb);
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
                        encode_srgb,
                        pad: 0,
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

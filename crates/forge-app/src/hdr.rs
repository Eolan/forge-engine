//! The HDR output (issue #94, D-022): which mode the shell presents in, what the demos' display
//! passes read of it ([`DisplayOutput`]), and the off-screen mode's preview pass.
//!
//! - HDR10 and scRGB present on the display, and only when the OS shows it in HDR (Windows'
//!   "Use HDR"), so scripted runs never switch the owner's monitor.
//! - Off-screen draws the frame into an HDR10 image and previews it on the SDR swapchain
//!   (`shaders/hdr_preview.slang`): everything but the present, on any monitor. Captures write
//!   the image's PQ codes as a 16-bit PNG beside the preview's.
//!
//! F2 turns HDR on and off, F3 steps the peak through ACES 2.0's presets, F4 switches the
//! preview to false colours.

use std::str::FromStr;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, DisplayCaps, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline,
    Result, ShaderCompiler, ShaderStage, vk,
};

/// The HDR output asked for (`--hdr`, `FORGE_HDR`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HdrMode {
    /// SDR.
    #[default]
    Off,
    /// HDR10 (Rec.2100 PQ) on the display, when the OS shows it in HDR.
    Hdr10,
    /// scRGB (linear half floats) on the display, when the OS shows it in HDR.
    ScRgb,
    /// HDR10 drawn off-screen and previewed on the SDR swapchain.
    Offscreen,
}

impl HdrMode {
    /// Short name for logs and the overlay.
    pub fn name(self) -> &'static str {
        match self {
            HdrMode::Off => "off",
            HdrMode::Hdr10 => "hdr10",
            HdrMode::ScRgb => "scrgb",
            HdrMode::Offscreen => "offscreen",
        }
    }
}

impl FromStr for HdrMode {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, Self::Err> {
        match text.to_ascii_lowercase().as_str() {
            "off" | "sdr" => Ok(HdrMode::Off),
            "hdr10" | "pq" => Ok(HdrMode::Hdr10),
            "scrgb" => Ok(HdrMode::ScRgb),
            "offscreen" => Ok(HdrMode::Offscreen),
            _ => Err(format!(
                "unknown HDR mode {text:?}: off, hdr10, scrgb or offscreen"
            )),
        }
    }
}

/// The format of the off-screen HDR10 image.
pub const OFFSCREEN_FORMAT: vk::Format = vk::Format::A2B10G10R10_UNORM_PACK32;

/// The ACES 2.0 peaks the HDR output steps through (F3), in nits: the Academy's presets.
pub const PEAKS: [f32; 4] = [500.0, 1000.0, 2000.0, 4000.0];

/// What the frame's target is and how HDR is set, for the demos' display passes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayOutput {
    /// The mode in use (off when HDR was asked for and is not available).
    pub mode: HdrMode,
    /// The frame target's format: the swapchain's, or the off-screen HDR image's.
    pub format: vk::Format,
    /// What the OS says of the window's display.
    pub caps: Option<DisplayCaps>,
    /// The peak the HDR output is made for, in nits (ACES 2.0's preset: the largest of
    /// [`PEAKS`] not above it).
    pub peak: f32,
    /// The paper-white offset: stops added to the scene before ACES 2.0. 0 is the Academy's
    /// look (the owner's pick, D-022).
    pub scene_stops: f32,
    /// The UI's white in nits: the OS's SDR white level when it shows HDR, BT.2408's 203
    /// nits otherwise.
    pub ui_white: f32,
}

impl DisplayOutput {
    /// SDR on a target of `format`, the HDR settings from `caps`.
    pub(crate) fn new(format: vk::Format, caps: Option<DisplayCaps>) -> Self {
        let mut output = Self {
            mode: HdrMode::Off,
            format,
            caps,
            peak: 1000.0,
            scene_stops: 0.0,
            ui_white: 203.0,
        };
        output.refresh(caps);
        output
    }

    /// Takes what the OS now says of the display: its peak (as a preset) and SDR white when
    /// it shows HDR, 1000 and 203 nits otherwise. The paper-white offset stays.
    pub(crate) fn refresh(&mut self, caps: Option<DisplayCaps>) {
        let hdr_on = caps.filter(|c| c.hdr_on);
        self.caps = caps;
        self.peak = hdr_on.and_then(|c| c.peak).map_or(1000.0, preset_peak);
        self.ui_white = hdr_on.map_or(203.0, |c| c.sdr_white);
    }

    /// Whether the target holds HDR.
    pub fn is_hdr(&self) -> bool {
        self.mode != HdrMode::Off
    }

    /// The overlay's line.
    pub(crate) fn describe(&self) -> String {
        let os = match self.caps {
            Some(c) => format!(
                "display: HDR {}, SDR white {:.0} nits, peak {}",
                if c.hdr_on { "on" } else { "off" },
                c.sdr_white,
                c.peak
                    .map_or("unknown".to_owned(), |p| format!("{p:.0} nits"))
            ),
            None => "display: unknown".to_owned(),
        };
        format!(
            "hdr: {} ({:?}), ACES 2.0 at {:.0} nits, paper white {:+.1} stops, UI {:.0} nits; {os}; F2 HDR, F3 peak, F4 false colours",
            self.mode.name(),
            self.format,
            self.peak,
            self.scene_stops,
            self.ui_white
        )
    }
}

/// The largest of [`PEAKS`] not above `nits` (the smallest for dimmer displays).
pub fn preset_peak(nits: f32) -> f32 {
    PEAKS
        .into_iter()
        .rev()
        .find(|&p| p <= nits)
        .unwrap_or(PEAKS[0])
}

/// The next of [`PEAKS`] after `peak`.
pub(crate) fn next_peak(peak: f32) -> f32 {
    let i = PEAKS
        .iter()
        .position(|&p| p == peak)
        .map_or(0, |i| (i + 1) % PEAKS.len());
    PEAKS[i]
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PreviewPush {
    image: u32,
    mode: u32,
    paper_white: f32,
    peak: f32,
}

/// The off-screen mode's preview pass.
pub(crate) struct Preview {
    pipeline: Pipeline,
    /// False colours (F4).
    pub false_colours: bool,
}

impl Preview {
    pub(crate) fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("hdr_preview.slang", "vert_main", ShaderStage::Vertex)?,
            "hdr preview vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("hdr_preview.slang", "frag_main", ShaderStage::Fragment)?,
            "hdr preview fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[format],
            push_constant_bytes: std::mem::size_of::<PreviewPush>() as u32,
            alpha_blend: false,
            depth_test: None,
            depth_write: false,
            name: "hdr preview",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        Ok(Self {
            pipeline: pipeline?,
            // `FORGE_HDR_FALSE_COLOURS=1` starts with them (scripted captures).
            false_colours: std::env::var_os("FORGE_HDR_FALSE_COLOURS").is_some_and(|v| v != "0"),
        })
    }

    /// Declares "post/hdr preview": `hdr` (HDR10) shown on `target` (SDR).
    pub(crate) fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        hdr: ImageHandle,
        target: ImageHandle,
        extent: vk::Extent2D,
        output: &DisplayOutput,
    ) {
        let pipeline = &self.pipeline;
        let mode = u32::from(self.false_colours);
        let (paper_white, peak) = (output.ui_white, output.peak);
        graph
            .pass("post/hdr preview")
            .image(
                hdr,
                ImageAccess::Sampled(vk::PipelineStageFlags2::FRAGMENT_SHADER),
            )
            .image(target, ImageAccess::ColorAttachment)
            .run(move |resources, commands| {
                let attachments = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(target))
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
                    &PreviewPush {
                        image: resources.sampled(hdr).0,
                        mode,
                        paper_white,
                        peak,
                    },
                );
                commands.draw(3, 1);
                commands.end_rendering();
                Ok(())
            });
    }
}

/// A2B10G10R10 pixels (red in the low bits) as 16-bit RGB: each 10-bit code scaled to 16
/// bits exactly (`c << 6 | c >> 4`), so a 16-bit PNG holds the codes themselves.
pub(crate) fn a2b10g10r10_to_rgb16(pixels: &[u8]) -> Vec<u16> {
    let widen = |c: u32| ((c << 6) | (c >> 4)) as u16;
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|px| {
            let v = u32::from_le_bytes(*px);
            [
                widen(v & 0x3ff),
                widen((v >> 10) & 0x3ff),
                widen((v >> 20) & 0x3ff),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse() {
        assert_eq!("HDR10".parse::<HdrMode>(), Ok(HdrMode::Hdr10));
        assert_eq!("pq".parse::<HdrMode>(), Ok(HdrMode::Hdr10));
        assert_eq!("offscreen".parse::<HdrMode>(), Ok(HdrMode::Offscreen));
        assert!("hdr".parse::<HdrMode>().is_err());
    }

    #[test]
    fn peaks_follow_the_display() {
        assert_eq!(preset_peak(400.0), 500.0);
        assert_eq!(preset_peak(1015.0), 1000.0);
        assert_eq!(preset_peak(1999.0), 1000.0);
        assert_eq!(next_peak(4000.0), 500.0);
        assert_eq!(next_peak(500.0), 1000.0);
    }

    #[test]
    fn ten_bit_codes_widen_exactly() {
        // Red 0x3ff, green 0, blue 0x200, alpha 3.
        let v: u32 = 0x3ff | (0x200 << 20) | (3 << 30);
        let rgb = a2b10g10r10_to_rgb16(&v.to_le_bytes());
        assert_eq!(rgb, vec![0xffff, 0, 0x8020]);
        assert_eq!(rgb[2] >> 6, 0x200, "the code is the top 10 bits");
    }

    #[test]
    fn the_output_follows_the_os() {
        let caps = DisplayCaps {
            hdr_on: true,
            hdr_supported: true,
            sdr_white: 240.0,
            peak: Some(1015.0),
            full_frame_peak: Some(400.0),
            black: Some(0.01),
        };
        let output = DisplayOutput::new(vk::Format::B8G8R8A8_SRGB, Some(caps));
        assert_eq!((output.peak, output.ui_white), (1000.0, 240.0));
        assert_eq!(output.scene_stops, 0.0, "the Academy's look");
        let off = DisplayOutput::new(
            vk::Format::B8G8R8A8_SRGB,
            Some(DisplayCaps {
                hdr_on: false,
                ..caps
            }),
        );
        assert_eq!((off.peak, off.ui_white), (1000.0, 203.0));
        assert!(!off.is_hdr());
    }
}

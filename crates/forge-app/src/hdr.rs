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
//! preview to false colours, F5 opens the calibration pages (`calibration.rs`, issue #125).
//! The display's metadata takes MaxCLL and MaxFALL from the frames shown (`content_light.rs`).

use std::str::FromStr;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, DisplayCaps, FrameGraph, FullscreenPipelineDesc, HdrMetadata, ImageAccess, ImageHandle,
    Pipeline, Result, ShaderCompiler, ShaderStage, vk,
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

/// What the calibration pages set of the display (issue #125), in nits; `None` takes what the
/// OS says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisplaySettings {
    /// The brightest the display shows (HGiG's MaxTML).
    pub peak: Option<f32>,
    /// The darkest level at which it still shows detail (HGiG's MinTML).
    pub black: Option<f32>,
    /// The UI's white (paper white).
    pub ui_white: Option<f32>,
}

/// MaxCLL and MaxFALL of the frames shown (issue #125), in nits: the brightest pixel's largest
/// channel, and the largest frame average of the pixels' largest channels (CTA-861.3, over
/// Rec.2020).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContentLight {
    /// MaxCLL.
    pub max_cll: f32,
    /// MaxFALL.
    pub max_fall: f32,
}

impl ContentLight {
    /// Takes in a frame's values. Returns whether either grew by more than 1 %: worth telling
    /// the display again.
    pub(crate) fn grow(&mut self, frame: ContentLight) -> bool {
        let grew = frame.max_cll > self.max_cll * 1.01 || frame.max_fall > self.max_fall * 1.01;
        self.max_cll = self.max_cll.max(frame.max_cll);
        self.max_fall = self.max_fall.max(frame.max_fall);
        grew
    }
}

/// What the frame's target is and how HDR is set, for the demos' display passes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayOutput {
    /// The mode in use (off when HDR was asked for and is not available).
    pub mode: HdrMode,
    /// The frame target's format: the swapchain's, or the off-screen HDR image's.
    pub format: vk::Format,
    /// What the OS says of the window's display.
    pub caps: Option<DisplayCaps>,
    /// What the calibration pages set (issue #125), over what the OS says.
    pub settings: DisplaySettings,
    /// The display's peak in nits: the calibration's, else the OS's when it shows HDR, else
    /// 1000.
    pub display_peak: f32,
    /// The display's black in nits: the calibration's, else the OS's when it shows HDR, else
    /// 0.005.
    pub black: f32,
    /// The peak the HDR output is made for, in nits (ACES 2.0's preset: the largest of
    /// [`PEAKS`] not above the display's).
    pub peak: f32,
    /// The paper-white offset: stops added to the scene before ACES 2.0. 0 is the Academy's
    /// look (the owner's pick, D-022).
    pub scene_stops: f32,
    /// The UI's white in nits: the calibration's, else the OS's SDR white level when it shows
    /// HDR, else BT.2408's 203 nits.
    pub ui_white: f32,
    /// MaxCLL and MaxFALL of the frames shown since the mode or the peak last changed; `None`
    /// before the first is measured.
    pub content_light: Option<ContentLight>,
}

impl DisplayOutput {
    /// SDR on a target of `format`, the HDR settings from `caps`.
    pub(crate) fn new(format: vk::Format, caps: Option<DisplayCaps>) -> Self {
        let mut output = Self {
            mode: HdrMode::Off,
            format,
            caps,
            settings: DisplaySettings::default(),
            display_peak: 1000.0,
            black: 0.005,
            peak: 1000.0,
            scene_stops: 0.0,
            ui_white: 203.0,
            content_light: None,
        };
        output.refresh(caps);
        output
    }

    /// Takes what the OS now says of the display, under the calibration's values: the peak
    /// (and its preset), the black and the UI's white. The paper-white offset stays.
    pub(crate) fn refresh(&mut self, caps: Option<DisplayCaps>) {
        let hdr_on = caps.filter(|c| c.hdr_on);
        self.caps = caps;
        self.display_peak = self
            .settings
            .peak
            .or(hdr_on.and_then(|c| c.peak))
            .unwrap_or(1000.0);
        self.peak = preset_peak(self.display_peak);
        self.black = self
            .settings
            .black
            .or(hdr_on.and_then(|c| c.black))
            .unwrap_or(0.005);
        self.ui_white = self
            .settings
            .ui_white
            .or(hdr_on.map(|c| c.sdr_white))
            .unwrap_or(203.0);
    }

    /// The OS's values alone, without the calibration's: what Backspace goes back to on a
    /// calibration page.
    pub(crate) fn os_values(&self) -> DisplayOutput {
        let mut os = DisplayOutput {
            settings: DisplaySettings::default(),
            ..*self
        };
        os.refresh(self.caps);
        os
    }

    /// Whether the target holds HDR.
    pub fn is_hdr(&self) -> bool {
        self.mode != HdrMode::Off
    }

    /// What the display is told of the content: the preset's peak, the black, and MaxCLL and
    /// MaxFALL once measured (the peak and 0, unknown, before).
    pub fn metadata(&self) -> HdrMetadata {
        HdrMetadata {
            peak: self.peak,
            black: self.black,
            max_cll: self.content_light.map_or(self.peak, |c| c.max_cll),
            max_fall: self.content_light.map_or(0.0, |c| c.max_fall),
        }
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
        let calibrated = if self.settings == DisplaySettings::default() {
            ""
        } else {
            " (calibrated)"
        };
        let light = self.content_light.map_or(String::new(), |c| {
            format!(", MaxCLL {:.0} MaxFALL {:.0} nits", c.max_cll, c.max_fall)
        });
        format!(
            "hdr: {} ({:?}), ACES 2.0 at {:.0} nits, paper white {:+.1} stops, UI {:.0} nits, peak {:.0} black {:.4} nits{calibrated}{light}; {os}; F2 HDR, F3 peak, F4 false colours, F5 calibrate",
            self.mode.name(),
            self.format,
            self.peak,
            self.scene_stops,
            self.ui_white,
            self.display_peak,
            self.black
        )
    }
}

/// The PQ signal (SMPTE ST 2084) of `nits`, in [0, 1].
pub fn pq_encode(nits: f32) -> f32 {
    let (m1, m2, c1, c2, c3) = PQ;
    let p = (nits / 10000.0).clamp(0.0, 1.0).powf(m1);
    ((c1 + c2 * p) / (1.0 + c3 * p)).powf(m2)
}

/// The nits of the PQ signal `signal` (SMPTE ST 2084).
pub fn pq_decode(signal: f32) -> f32 {
    let (m1, m2, c1, c2, c3) = PQ;
    let e = signal.clamp(0.0, 1.0).powf(1.0 / m2);
    ((e - c1).max(0.0) / (c2 - c3 * e)).powf(1.0 / m1) * 10000.0
}

/// ST 2084's m1, m2, c1, c2 and c3.
const PQ: (f32, f32, f32, f32, f32) = (
    2610.0 / 16384.0,
    2523.0 / 4096.0 * 128.0,
    3424.0 / 4096.0,
    2413.0 / 4096.0 * 32.0,
    2392.0 / 4096.0 * 32.0,
);

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

    #[test]
    fn pq_gives_the_known_codes() {
        // docs/research/hdr-output.md: 497 / 520 / 594 / 769 / 923 of 1023 for 80 / 100 / 203 /
        // 1000 / 4000 nits.
        for (nits, code) in [
            (80.0, 497.0),
            (100.0, 520.0),
            (203.0, 594.0),
            (1000.0, 769.0),
            (4000.0, 923.0),
        ] {
            assert_eq!((pq_encode(nits) * 1023.0).round(), code, "{nits}");
            assert!((pq_decode(pq_encode(nits)) / nits - 1.0).abs() < 1e-4);
        }
        // Black is a hair above 0 (c1^m2), well inside code 0.
        assert!(pq_encode(0.0) < 1e-6 && pq_decode(0.0) == 0.0);
        assert_eq!(pq_encode(10000.0), 1.0);
    }

    #[test]
    fn the_calibration_goes_over_the_os() {
        let caps = DisplayCaps {
            hdr_on: true,
            hdr_supported: true,
            sdr_white: 240.0,
            peak: Some(1015.0),
            full_frame_peak: Some(400.0),
            black: Some(0.01),
        };
        let mut output = DisplayOutput::new(vk::Format::A2B10G10R10_UNORM_PACK32, Some(caps));
        assert_eq!((output.display_peak, output.black), (1015.0, 0.01));
        output.settings = DisplaySettings {
            peak: Some(2400.0),
            black: None,
            ui_white: Some(160.0),
        };
        output.refresh(Some(caps));
        assert_eq!(
            (
                output.display_peak,
                output.peak,
                output.black,
                output.ui_white
            ),
            (2400.0, 2000.0, 0.01, 160.0)
        );
        let os = output.os_values();
        assert_eq!((os.peak, os.ui_white), (1000.0, 240.0));
        // The metadata: the preset's peak and the black; MaxCLL the peak until measured.
        let metadata = output.metadata();
        assert_eq!((metadata.peak, metadata.black), (2000.0, 0.01));
        assert_eq!((metadata.max_cll, metadata.max_fall), (2000.0, 0.0));
        output.content_light = Some(ContentLight {
            max_cll: 812.0,
            max_fall: 64.0,
        });
        assert_eq!(
            (output.metadata().max_cll, output.metadata().max_fall),
            (812.0, 64.0)
        );
    }

    #[test]
    fn the_content_light_keeps_the_largest() {
        let mut light = ContentLight {
            max_cll: 500.0,
            max_fall: 50.0,
        };
        let frame = |max_cll, max_fall| ContentLight { max_cll, max_fall };
        assert!(!light.grow(frame(400.0, 50.2)), "within 1 %");
        assert!(light.grow(frame(600.0, 20.0)));
        assert_eq!((light.max_cll, light.max_fall), (600.0, 50.2));
    }
}

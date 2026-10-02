//! The HDR calibration pages (issue #125, D-022; `shaders/hdr_calibration.slang`): three
//! full-screen pages in Unity's and HGiG's form that set what the display shows, over what the
//! OS says of it.
//! - Peak: a mark inside a square of a tenth of the screen at the signal's top. Raised until it
//!   disappears, it is the brightest the display shows (HGiG's MaxTML), and picks ACES 2.0's
//!   preset (the largest not above it).
//! - Black: a mark on black, at the darkest value it still shows (MinTML), for the metadata.
//! - Paper white: half the screen at the peak, half black, the mark at the UI's white.
//!
//! F5 opens them in an HDR mode and steps through them. Up and Down move the value by 4 PQ
//! codes (1 with Shift), Backspace goes back to the OS's value, Escape leaves with the values
//! of before. Each value applies at once. Leaving the last page with F5 saves them per monitor
//! in `settings/display.txt` (interactive runs only: scripted runs neither read nor write it,
//! so captures do not depend on it).

use std::path::PathBuf;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use winit::keyboard::KeyCode;

use crate::hdr::{DisplayOutput, DisplaySettings, pq_decode, pq_encode};
use crate::overlay::{Canvas, Color};

/// A calibration page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    /// The brightest the display shows.
    Peak,
    /// The darkest level at which it still shows detail.
    Black,
    /// The UI's white.
    PaperWhite,
}

impl Page {
    /// The page named `name` (`FORGE_HDR_CALIBRATION`: peak, black or white).
    pub(crate) fn parse(name: &str) -> Option<Page> {
        match name {
            "peak" => Some(Page::Peak),
            "black" => Some(Page::Black),
            "white" | "paper-white" => Some(Page::PaperWhite),
            _ => None,
        }
    }

    fn next(self) -> Option<Page> {
        match self {
            Page::Peak => Some(Page::Black),
            Page::Black => Some(Page::PaperWhite),
            Page::PaperWhite => None,
        }
    }

    fn index(self) -> u32 {
        match self {
            Page::Peak => 0,
            Page::Black => 1,
            Page::PaperWhite => 2,
        }
    }

    /// The values the page offers, in nits.
    fn range(self) -> (f32, f32) {
        match self {
            Page::Peak => (100.0, 10000.0),
            Page::Black => (0.0, 5.0),
            Page::PaperWhite => (40.0, 1000.0),
        }
    }

    /// The page's value in `output`.
    fn value(self, output: &DisplayOutput) -> f32 {
        match self {
            Page::Peak => output.display_peak,
            Page::Black => output.black,
            Page::PaperWhite => output.ui_white,
        }
    }

    /// Sets the page's value in `settings` (`None`: the OS's).
    fn set(self, settings: &mut DisplaySettings, value: Option<f32>) {
        match self {
            Page::Peak => settings.peak = value,
            Page::Black => settings.black = value,
            Page::PaperWhite => settings.ui_white = value,
        }
    }
}

/// `nits` moved by `codes` 10-bit PQ codes, kept within `range`: always on a code, so the mark
/// shows exactly the value set.
pub(crate) fn step(nits: f32, codes: i32, range: (f32, f32)) -> f32 {
    let code = |nits: f32| (pq_encode(nits) * 1023.0).round() as i32;
    let moved = (code(nits) + codes).clamp(code(range.0), code(range.1));
    pq_decode(moved as f32 / 1023.0)
}

/// What a key did on the pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Nothing the shell has to follow.
    None,
    /// The display's values changed.
    Changed,
    /// The pages closed: with F5 past the last one (`save`), or with Escape (the values of
    /// before are back, changed or not).
    Closed {
        /// Whether to keep the values.
        save: bool,
    },
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CalibrationPush {
    page: u32,
    encoding: u32,
    value: f32,
    peak: f32,
    width: u32,
    height: u32,
}

/// The pages' state and pass.
pub(crate) struct Calibration {
    /// The pass per target format, made when first needed.
    pipelines: Vec<(vk::Format, Pipeline)>,
    page: Option<Page>,
    /// The settings when the pages opened (Escape goes back to them).
    before: DisplaySettings,
}

impl Calibration {
    pub(crate) fn new() -> Self {
        Self {
            pipelines: Vec::new(),
            page: None,
            before: DisplaySettings::default(),
        }
    }

    /// The page shown, if any.
    pub(crate) fn page(&self) -> Option<Page> {
        self.page
    }

    /// Opens the pages at `page` over `output` (compiling the pass for its target).
    pub(crate) fn open(
        &mut self,
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        output: &DisplayOutput,
        page: Page,
    ) -> Result<()> {
        self.prepare(device, shaders, output.format)?;
        self.before = output.settings;
        self.page = Some(page);
        tracing::info!(?page, "HDR calibration");
        Ok(())
    }

    /// Compiles the pass for targets of `format` unless it was already.
    pub(crate) fn prepare(
        &mut self,
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
    ) -> Result<()> {
        if self.pipelines.iter().any(|(f, _)| *f == format) {
            return Ok(());
        }
        let vertex = device.create_shader_module(
            &shaders.compile("hdr_calibration.slang", "vert_main", ShaderStage::Vertex)?,
            "hdr calibration vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("hdr_calibration.slang", "frag_main", ShaderStage::Fragment)?,
            "hdr calibration fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[format],
            push_constant_bytes: std::mem::size_of::<CalibrationPush>() as u32,
            alpha_blend: false,
            depth_test: None,
            depth_write: false,
            name: "hdr calibration",
        });
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        self.pipelines.push((format, pipeline?));
        Ok(())
    }

    /// Closes the pages, keeping the values set.
    pub(crate) fn close(&mut self) {
        self.page = None;
    }

    /// Handles `code` on the open pages (`fine`: Shift is down), changing `output`'s settings.
    pub(crate) fn key(&mut self, code: KeyCode, fine: bool, output: &mut DisplayOutput) -> Action {
        let Some(page) = self.page else {
            return Action::None;
        };
        let codes = if fine { 1 } else { 4 };
        let value = match code {
            KeyCode::ArrowUp => Some(step(page.value(output), codes, page.range())),
            KeyCode::ArrowDown => Some(step(page.value(output), -codes, page.range())),
            KeyCode::Backspace => None,
            KeyCode::F5 => {
                self.page = page.next();
                return match self.page {
                    Some(next) => {
                        tracing::info!(page = ?next, "HDR calibration");
                        Action::None
                    }
                    None => Action::Closed { save: true },
                };
            }
            KeyCode::Escape => {
                output.settings = self.before;
                output.refresh(output.caps);
                self.page = None;
                return Action::Closed { save: false };
            }
            _ => return Action::None,
        };
        page.set(&mut output.settings, value);
        output.refresh(output.caps);
        Action::Changed
    }

    /// Declares "app/hdr calibration": the page drawn over `target` (an HDR10 or scRGB image).
    pub(crate) fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        target: ImageHandle,
        extent: vk::Extent2D,
        output: &DisplayOutput,
    ) {
        let Some(page) = self.page else {
            return;
        };
        let Some((_, pipeline)) = self.pipelines.iter().find(|(f, _)| *f == output.format) else {
            return;
        };
        let push = CalibrationPush {
            page: page.index(),
            // `OUTPUT_PQ` or `OUTPUT_SCRGB` of `tonemap.slang`.
            encoding: if output.format == vk::Format::R16G16B16A16_SFLOAT {
                3
            } else {
                2
            },
            value: page.value(output),
            peak: output.display_peak,
            width: extent.width,
            height: extent.height,
        };
        graph
            .pass("app/hdr calibration")
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
                commands.push_constants(pipeline, &push);
                commands.draw(3, 1);
                commands.end_rendering();
                Ok(())
            });
    }

    /// The page's instructions and values at the bottom of `canvas`.
    pub(crate) fn layout(&self, canvas: &mut Canvas, output: &DisplayOutput) {
        let Some(page) = self.page else {
            return;
        };
        let os = output.os_values();
        let value = page.value(output);
        let code = (pq_encode(value) * 1023.0).round();
        let (title, how, values) = match page {
            Page::Peak => (
                "HDR calibration, 1 of 3: peak",
                "Raise the mark (Up) until it disappears into the white square.",
                format!(
                    "peak {value:.0} nits (PQ code {code}), the OS's {:.0}; ACES 2.0's preset {:.0} nits",
                    os.display_peak, output.peak
                ),
            ),
            Page::Black => (
                "HDR calibration, 2 of 3: black",
                "Lower the mark (Down) until it disappears, then raise it until it just shows.",
                format!(
                    "black {value:.4} nits (PQ code {code}), the OS's {:.4}",
                    os.black
                ),
            ),
            Page::PaperWhite => (
                "HDR calibration, 3 of 3: paper white",
                "Set the mark as bright as a sheet of paper: the UI's white, comfortable on black.",
                format!(
                    "UI white {value:.0} nits (PQ code {code}), the OS's {:.0}",
                    os.ui_white
                ),
            ),
        };
        let keys = if page == Page::PaperWhite {
            "Up/Down: 4 codes (Shift: 1)   Backspace: the OS's value   F5: save   Esc: cancel"
        } else {
            "Up/Down: 4 codes (Shift: 1)   Backspace: the OS's value   F5: next page   Esc: cancel"
        };
        // On the black page the text stays dim, out of the way of the eye's adaptation.
        let (bright, normal) = if page == Page::Black {
            (Color::Dim, Color::Dim)
        } else {
            (Color::White, Color::Grey)
        };
        let lines = [
            (title, bright),
            (how, normal),
            (values.as_str(), bright),
            (keys, normal),
        ];
        let width = lines.iter().map(|(t, _)| t.len()).max().unwrap_or(0) + 4;
        let col = canvas.cols().saturating_sub(width) / 2;
        let row = canvas.rows().saturating_sub(lines.len() + 3);
        canvas.panel(col, row, width, lines.len() + 2, Color::PanelDark);
        for (i, (text, color)) in lines.into_iter().enumerate() {
            canvas.text(col + 2, row + 1 + i, text, color);
        }
    }
}

/// The calibration saved per monitor, `settings/display.txt` in the workspace: a line per
/// monitor, its OS name then `peak=`, `black=` and `ui_white=` in nits, a value left out
/// following the OS.
pub(crate) struct SettingsFile {
    path: PathBuf,
}

impl SettingsFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// The settings saved for `monitor` (none when the file or its line is missing).
    pub(crate) fn load(&self, monitor: &str) -> DisplaySettings {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let settings = text
            .lines()
            .filter_map(parse_line)
            .find(|(name, _)| *name == monitor)
            .map(|(_, settings)| settings)
            .unwrap_or_default();
        if settings != DisplaySettings::default() {
            tracing::info!(monitor, ?settings, path = %self.path.display(), "HDR calibration loaded");
        }
        settings
    }

    /// Saves `settings` for `monitor`, keeping the other monitors' lines.
    pub(crate) fn save(&self, monitor: &str, settings: DisplaySettings) -> std::io::Result<()> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, with_line(&text, monitor, settings))?;
        tracing::info!(monitor, ?settings, path = %self.path.display(), "HDR calibration saved");
        Ok(())
    }
}

const HEADER: &str = "# Forge's HDR calibration (F5 in a demo, issue #125): a line per monitor, the peak, the\n# black and the UI's white in nits; a value left out follows the OS. Delete a line to forget it.\n";

/// A monitor's name and settings from a line of the file.
fn parse_line(line: &str) -> Option<(&str, DisplaySettings)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut words = line.split_whitespace();
    let name = words.next()?;
    let mut settings = DisplaySettings::default();
    for word in words {
        let Some((key, value)) = word.split_once('=') else {
            continue;
        };
        let value = value.parse::<f32>().ok().filter(|v| v.is_finite());
        match key {
            "peak" => settings.peak = value,
            "black" => settings.black = value,
            "ui_white" => settings.ui_white = value,
            _ => {}
        }
    }
    Some((name, settings))
}

/// `text` with `monitor`'s line replaced by `settings` (added when missing; removed when they
/// all follow the OS).
fn with_line(text: &str, monitor: &str, settings: DisplaySettings) -> String {
    let mut out = String::new();
    if !text.starts_with('#') {
        out.push_str(HEADER);
    }
    for line in text.lines() {
        if parse_line(line).is_some_and(|(name, _)| name == monitor) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if settings != DisplaySettings::default() {
        out.push_str(monitor);
        for (key, value) in [
            ("peak", settings.peak),
            ("black", settings.black),
            ("ui_white", settings.ui_white),
        ] {
            if let Some(value) = value {
                out.push_str(&format!(" {key}={value}"));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdr::HdrMode;
    use forge_gpu::DisplayCaps;

    #[test]
    fn steps_land_on_pq_codes_within_the_range() {
        // 1000 nits is code 769 (rounded); four codes up and one down.
        let up = step(1000.0, 4, Page::Peak.range());
        assert_eq!((pq_encode(up) * 1023.0).round(), 773.0);
        let back = step(up, -1, Page::Peak.range());
        assert_eq!((pq_encode(back) * 1023.0).round(), 772.0);
        assert!((pq_encode(back) * 1023.0 - 772.0).abs() < 1e-2, "on a code");
        // The range holds at both ends.
        assert_eq!(step(9000.0, 100, Page::Peak.range()), 10000.0);
        // 0.001 nits is code 6.
        assert!(step(0.001, -4, Page::Black.range()) > 0.0);
        assert!(step(0.001, -8, Page::Black.range()) == 0.0);
        let low = step(50.0, -100, Page::PaperWhite.range());
        assert!((39.0..41.0).contains(&low), "{low}");
    }

    #[test]
    fn keys_set_the_values_at_once_and_escape_restores_them() {
        let caps = DisplayCaps {
            hdr_on: true,
            hdr_supported: true,
            sdr_white: 240.0,
            peak: Some(1015.0),
            full_frame_peak: Some(400.0),
            black: Some(0.01),
        };
        let mut output = DisplayOutput::new(vk::Format::A2B10G10R10_UNORM_PACK32, Some(caps));
        output.mode = HdrMode::Hdr10;
        let mut pages = Calibration::new();
        pages.before = output.settings;
        pages.page = Some(Page::Peak);
        // Down from 1015 nits below 1000: the 500-nit preset.
        for _ in 0..3 {
            assert_eq!(
                pages.key(KeyCode::ArrowDown, false, &mut output),
                Action::Changed
            );
        }
        assert!(
            output.display_peak < 1000.0 && output.peak == 500.0,
            "{output:?}"
        );
        assert_eq!(
            pages.key(KeyCode::Backspace, false, &mut output),
            Action::Changed
        );
        assert_eq!((output.display_peak, output.peak), (1015.0, 1000.0));
        assert_eq!(pages.key(KeyCode::F5, false, &mut output), Action::None);
        assert_eq!(pages.page(), Some(Page::Black));
        pages.key(KeyCode::ArrowUp, true, &mut output);
        assert!(output.black > 0.01, "{output:?}");
        pages.key(KeyCode::F5, false, &mut output);
        pages.key(KeyCode::ArrowUp, false, &mut output);
        assert!(output.ui_white > 240.0);
        assert_eq!(
            pages.key(KeyCode::Escape, false, &mut output),
            Action::Closed { save: false }
        );
        assert_eq!(output.settings, DisplaySettings::default());
        assert_eq!((output.black, output.ui_white), (0.01, 240.0));
        assert_eq!(pages.page(), None);
    }

    #[test]
    fn the_file_keeps_a_line_per_monitor() {
        let a = DisplaySettings {
            peak: Some(812.5),
            black: None,
            ui_white: Some(240.0),
        };
        let b = DisplaySettings {
            black: Some(0.02),
            ..DisplaySettings::default()
        };
        let text = with_line("", r"\\.\DISPLAY1", a);
        assert!(text.starts_with('#'));
        let text = with_line(&text, r"\\.\DISPLAY2", b);
        let lines: Vec<_> = text.lines().filter_map(parse_line).collect();
        assert_eq!(lines, vec![(r"\\.\DISPLAY1", a), (r"\\.\DISPLAY2", b)]);
        // Saving again replaces the line; settings that all follow the OS remove it.
        let text = with_line(&text, r"\\.\DISPLAY1", b);
        let text = with_line(&text, r"\\.\DISPLAY2", DisplaySettings::default());
        let lines: Vec<_> = text.lines().filter_map(parse_line).collect();
        assert_eq!(lines, vec![(r"\\.\DISPLAY1", b)]);
        assert_eq!(text.matches("# Forge").count(), 1);
    }

    #[test]
    fn saved_settings_load_back() {
        let dir = std::env::temp_dir().join(format!("forge-settings-{}", std::process::id()));
        let file = SettingsFile::new(dir.join("settings/display.txt"));
        assert_eq!(
            file.load("default"),
            DisplaySettings::default(),
            "no file yet"
        );
        let settings = DisplaySettings {
            peak: Some(step(812.0, 0, Page::Peak.range())),
            black: Some(step(0.02, 0, Page::Black.range())),
            ui_white: None,
        };
        file.save("default", settings).expect("saved");
        assert_eq!(file.load("default"), settings);
        assert_eq!(file.load("other"), DisplaySettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

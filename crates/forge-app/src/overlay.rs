//! The debug overlay: text and bars composited over the swapchain image
//! (`shaders/overlay.slang`).
//!
//! A grid of character cells is filled on the CPU every frame through a [`Canvas`] and drawn
//! by one full-screen triangle that reads the cells through a device address and the glyphs
//! from a font atlas, like every other pass. The atlas is rasterised at start-up from a TTF
//! (`FORGE_OVERLAY_FONT`, JetBrains Mono from `assets/fonts` by default, `FORGE_OVERLAY_FONT_PX`
//! for the size) with a built-in 5×7 pixel font as the fallback. No UI library: this is the
//! always-available statistics and profiling view; an editor UI comes with the tools phase.

use std::path::Path;
use std::sync::Arc;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FullscreenPipelineDesc, GraphImage,
    ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};

/// Cells per slot buffer: enough for 4K at 8×16 (480 × 135).
const MAX_CELLS: usize = 480 * 136;
const BAR_GLYPH: u32 = 255;
const GLYPHS: usize = 95;

/// Overlay colours (indices into the palette in `overlay.slang`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Color {
    /// Transparent.
    None = 0,
    /// Translucent panel background.
    Panel = 1,
    /// Bright text.
    White = 2,
    /// Normal text.
    Grey = 3,
    /// Secondary text.
    Dim = 4,
    /// Headings.
    Yellow = 5,
    /// Warm bars.
    Orange = 6,
    /// Hot bars.
    Red = 7,
    /// Good.
    Green = 8,
    /// Cool bars.
    Cyan = 9,
    /// Links and keys.
    Blue = 10,
    /// Accents.
    Magenta = 11,
    /// Empty part of a bar.
    Track = 12,
    /// A darker panel.
    PanelDark = 13,
    /// Header band.
    Header = 14,
}

/// A grid of cells to draw into. Coordinates are in cells, (0, 0) top left.
pub struct Canvas {
    cols: usize,
    rows: usize,
    cells: Vec<u32>,
}

impl Canvas {
    fn new() -> Self {
        Self {
            cols: 0,
            rows: 0,
            cells: Vec::new(),
        }
    }

    fn reset(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.cells.clear();
        self.cells.resize(cols * rows, 0);
    }

    /// Cells per row.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Sets the background of a rectangle (clipped to the canvas).
    pub fn panel(&mut self, col: usize, row: usize, width: usize, height: usize, bg: Color) {
        for r in row..(row + height).min(self.rows) {
            for c in col..(col + width).min(self.cols) {
                let cell = &mut self.cells[r * self.cols + c];
                *cell = (*cell & !0xF000) | ((bg as u32) << 12);
            }
        }
    }

    /// Writes `text` (clipped), keeping the cells' background; returns the column after it.
    pub fn text(&mut self, col: usize, row: usize, text: &str, fg: Color) -> usize {
        if row >= self.rows {
            return col;
        }
        let mut c = col;
        for ch in text.chars() {
            if c >= self.cols {
                break;
            }
            let code = ch as u32;
            let glyph = if (32..127).contains(&code) {
                code
            } else {
                u32::from(b'?')
            };
            let cell = &mut self.cells[row * self.cols + c];
            *cell = (*cell & 0xF000) | glyph | ((fg as u32) << 8);
            c += 1;
        }
        c
    }

    /// A horizontal bar of `width` cells filled to `fraction` (0..1) in `color`.
    pub fn bar(&mut self, col: usize, row: usize, width: usize, fraction: f32, color: Color) {
        if row >= self.rows {
            return;
        }
        let total = (fraction.clamp(0.0, 1.0) * width as f32 * 255.0) as u32;
        for i in 0..width {
            let c = col + i;
            if c >= self.cols {
                break;
            }
            let fill = total.saturating_sub(i as u32 * 255).min(255);
            let cell = &mut self.cells[row * self.cols + c];
            *cell = (*cell & 0xF000) | BAR_GLYPH | (fill << 16) | ((color as u32) << 24);
        }
    }
}

#[cfg(test)]
impl Canvas {
    /// A blank canvas of `cols` × `rows` cells.
    pub(crate) fn blank(cols: usize, rows: usize) -> Self {
        let mut canvas = Self::new();
        canvas.reset(cols, rows);
        canvas
    }

    /// The text of `row` (bars and empty cells as spaces) and the colour of its first
    /// character.
    pub(crate) fn row_text(&self, row: usize) -> (String, Option<u32>) {
        let cells = &self.cells[row * self.cols..(row + 1) * self.cols];
        let glyph = |cell: u32| cell & 0xFF;
        let text = cells
            .iter()
            .map(|&cell| match glyph(cell) {
                g @ 33..127 => char::from(g as u8),
                _ => ' ',
            })
            .collect::<String>();
        let first = cells
            .iter()
            .find(|&&cell| (33..127).contains(&glyph(cell)))
            .map(|cell| (cell >> 8) & 0xF);
        (text.trim_end().to_owned(), first)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    cells: u64,
    cols: u32,
    rows: u32,
    cell_w: u32,
    cell_h: u32,
    atlas: u32,
    pad: u32,
}

/// The overlay pass, its font atlas and per-slot cell buffers.
pub struct Overlay {
    pipeline: Pipeline,
    atlas: GraphImage,
    cells: Vec<Buffer>,
    canvas: Canvas,
    cell_w: u32,
    cell_h: u32,
}

impl Overlay {
    /// Compiles the pass for a colour attachment of `format`, rasterises the font (`font` at
    /// `font_px` pixels, or the built-in pixel font when missing or unreadable) and allocates
    /// the buffers.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        format: vk::Format,
        font: Option<&Path>,
        font_px: f32,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("overlay.slang", "vert_main", ShaderStage::Vertex)?,
            "overlay vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("overlay.slang", "frag_main", ShaderStage::Fragment)?,
            "overlay fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[format],
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            alpha_blend: true,
            depth_test: None,
            depth_write: false,
            name: "overlay",
        })?;
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);

        let atlas = font
            .and_then(|path| match std::fs::read(path) {
                Ok(bytes) => rasterize_ttf(&bytes, font_px),
                Err(e) => {
                    tracing::warn!(path = %path.display(), "overlay font not readable ({e}); using the built-in font");
                    None
                }
            })
            .unwrap_or_else(rasterize_builtin);
        let image = GraphImage::uploaded(
            device,
            ImageDesc {
                width: atlas.cell_w * GLYPHS as u32,
                height: atlas.cell_h,
                format: vk::Format::R8_UNORM,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "overlay font atlas",
            },
            &atlas.pixels,
        )?;
        let cells = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (MAX_CELLS * 4) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("overlay cells {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tracing::info!(
            cell_w = atlas.cell_w,
            cell_h = atlas.cell_h,
            "overlay font ready"
        );
        Ok(Self {
            pipeline,
            atlas: image,
            cells,
            canvas: Canvas::new(),
            cell_w: atlas.cell_w,
            cell_h: atlas.cell_h,
        })
    }

    /// Starts a frame's drawing: a cleared canvas sized to `extent`.
    pub fn begin(&mut self, extent: vk::Extent2D) -> &mut Canvas {
        let cols = ((extent.width / self.cell_w) as usize).max(1);
        let rows = ((extent.height / self.cell_h) as usize)
            .max(1)
            .min(MAX_CELLS / cols);
        self.canvas.reset(cols, rows);
        &mut self.canvas
    }

    /// Declares the pass that draws the canvas over `target` (blended over whatever the
    /// frame drew there).
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: usize,
        target: ImageHandle,
        extent: vk::Extent2D,
    ) {
        if self.canvas.cells.is_empty() {
            return;
        }
        let cells = &self.cells[slot];
        cells.write(0, &self.canvas.cells);
        let push = Push {
            cells: cells.address(),
            cols: self.canvas.cols as u32,
            rows: self.canvas.rows as u32,
            cell_w: self.cell_w,
            cell_h: self.cell_h,
            atlas: self.atlas.sampled().0,
            pad: 0,
        };
        let atlas = graph.import(&self.atlas);
        let pipeline = &self.pipeline;
        graph
            .pass("app/overlay")
            .image(target, ImageAccess::ColorAttachment)
            .image(
                atlas,
                ImageAccess::Sampled(vk::PipelineStageFlags2::FRAGMENT_SHADER),
            )
            .run(move |resources, commands| {
                let color = [vk::RenderingAttachmentInfo::default()
                    .image_view(resources.view(target))
                    .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .load_op(vk::AttachmentLoadOp::LOAD)
                    .store_op(vk::AttachmentStoreOp::STORE)];
                let info = vk::RenderingInfo::default()
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D::default(),
                        extent,
                    })
                    .layer_count(1)
                    .color_attachments(&color);
                commands.begin_rendering(&info);
                commands.bind_pipeline(pipeline);
                commands.set_viewport_full(extent);
                commands.push_constants(pipeline, &push);
                commands.draw(3, 1);
                commands.end_rendering();
                Ok(())
            });
    }
}

/// One row of 95 glyph cells, coverage 0..255.
struct FontAtlas {
    cell_w: u32,
    cell_h: u32,
    pixels: Vec<u8>,
}

impl FontAtlas {
    fn new(cell_w: u32, cell_h: u32) -> Self {
        Self {
            cell_w,
            cell_h,
            pixels: vec![0; (cell_w * GLYPHS as u32 * cell_h) as usize],
        }
    }

    fn put(&mut self, glyph: usize, x: i32, y: i32, coverage: u8) {
        if x < 0 || y < 0 || x >= self.cell_w as i32 || y >= self.cell_h as i32 {
            return;
        }
        let stride = self.cell_w as usize * GLYPHS;
        let index = y as usize * stride + glyph * self.cell_w as usize + x as usize;
        self.pixels[index] = self.pixels[index].max(coverage);
    }
}

/// Rasterises ASCII 32..127 of a TTF/OTF at `px` pixels into fixed cells (the widest advance,
/// so proportional fonts work too, just spaced).
fn rasterize_ttf(bytes: &[u8], px: f32) -> Option<FontAtlas> {
    let font = FontRef::try_from_slice(bytes).ok()?;
    // `px` is the em size; ab_glyph's `PxScale` is the ascent-to-descent height.
    let units_per_em = font.units_per_em().unwrap_or(1000.0);
    let scale = PxScale::from(px.max(6.0) * font.height_unscaled() / units_per_em);
    let scaled = font.as_scaled(scale);
    let advance = (32_u8..127)
        .map(|c| scaled.h_advance(scaled.glyph_id(char::from(c))))
        .fold(0.0_f32, f32::max);
    let ascent = scaled.ascent();
    let cell_w = advance.ceil().max(1.0) as u32;
    let cell_h = (scaled.height() + 1.0).ceil().max(1.0) as u32;
    let mut atlas = FontAtlas::new(cell_w, cell_h);
    for (i, code) in (32_u8..127).enumerate() {
        let mut glyph = scaled.scaled_glyph(char::from(code));
        glyph.position = point(0.0, ascent);
        if let Some(outlined) = scaled.font().outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|x, y, coverage| {
                let px = bounds.min.x.round() as i32 + x as i32;
                let py = bounds.min.y.round() as i32 + y as i32;
                atlas.put(i, px, py, (coverage.clamp(0.0, 1.0) * 255.0) as u8);
            });
        }
    }
    Some(atlas)
}

/// The built-in 5×7 pixel font drawn at 2× into 12×16 cells.
fn rasterize_builtin() -> FontAtlas {
    let mut atlas = FontAtlas::new(12, 16);
    for (i, rows) in FONT.iter().enumerate() {
        for (r, line) in rows.iter().enumerate() {
            for (k, ch) in line.bytes().take(5).enumerate() {
                if ch == b'#' {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            atlas.put(i, 1 + (k * 2 + dx) as i32, 1 + (r * 2 + dy) as i32, 255);
                        }
                    }
                }
            }
        }
    }
    atlas
}

/// ASCII 32..127, 5 columns × 7 rows, `#` = pixel.
#[rustfmt::skip]
const FONT: [[&str; 7]; GLYPHS] = [
    [".....", ".....", ".....", ".....", ".....", ".....", "....."], // space
    ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."], // !
    [".#.#.", ".#.#.", ".#.#.", ".....", ".....", ".....", "....."], // "
    [".#.#.", ".#.#.", "#####", ".#.#.", "#####", ".#.#.", ".#.#."], // #
    ["..#..", ".####", "#.#..", ".###.", "..#.#", "####.", "..#.."], // $
    ["##...", "##..#", "...#.", "..#..", ".#...", "#..##", "...##"], // %
    [".##..", "#..#.", "#.#..", ".#...", "#.#.#", "#..#.", ".##.#"], // &
    ["..#..", "..#..", ".#...", ".....", ".....", ".....", "....."], // '
    ["...#.", "..#..", ".#...", ".#...", ".#...", "..#..", "...#."], // (
    [".#...", "..#..", "...#.", "...#.", "...#.", "..#..", ".#..."], // )
    [".....", "..#..", "#.#.#", ".###.", "#.#.#", "..#..", "....."], // *
    [".....", "..#..", "..#..", "#####", "..#..", "..#..", "....."], // +
    [".....", ".....", ".....", ".....", ".##..", "..#..", ".#..."], // ,
    [".....", ".....", ".....", "#####", ".....", ".....", "....."], // -
    [".....", ".....", ".....", ".....", ".....", ".##..", ".##.."], // .
    [".....", "....#", "...#.", "..#..", ".#...", "#....", "....."], // /
    [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."], // 0
    ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."], // 1
    [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"], // 2
    ["#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###."], // 3
    ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."], // 4
    ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."], // 5
    ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."], // 6
    ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."], // 7
    [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."], // 8
    [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."], // 9
    [".....", ".##..", ".##..", ".....", ".##..", ".##..", "....."], // :
    [".....", ".##..", ".##..", ".....", ".##..", "..#..", ".#..."], // ;
    ["...#.", "..#..", ".#...", "#....", ".#...", "..#..", "...#."], // <
    [".....", ".....", "#####", ".....", "#####", ".....", "....."], // =
    [".#...", "..#..", "...#.", "....#", "...#.", "..#..", ".#..."], // >
    [".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."], // ?
    [".###.", "#...#", "....#", ".##.#", "#.#.#", "#.#.#", ".###."], // @
    [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"], // A
    ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."], // B
    [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."], // C
    ["###..", "#..#.", "#...#", "#...#", "#...#", "#..#.", "###.."], // D
    ["#####", "#....", "#....", "####.", "#....", "#....", "#####"], // E
    ["#####", "#....", "#....", "####.", "#....", "#....", "#...."], // F
    [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".####"], // G
    ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"], // H
    [".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."], // I
    ["..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##.."], // J
    ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"], // K
    ["#....", "#....", "#....", "#....", "#....", "#....", "#####"], // L
    ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"], // M
    ["#...#", "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#"], // N
    [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."], // O
    ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."], // P
    [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"], // Q
    ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"], // R
    [".####", "#....", "#....", ".###.", "....#", "....#", "####."], // S
    ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."], // T
    ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."], // U
    ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."], // V
    ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "#.#.#", ".#.#."], // W
    ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"], // X
    ["#...#", "#...#", "#...#", ".#.#.", "..#..", "..#..", "..#.."], // Y
    ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"], // Z
    [".###.", ".#...", ".#...", ".#...", ".#...", ".#...", ".###."], // [
    [".....", "#....", ".#...", "..#..", "...#.", "....#", "....."], // backslash
    [".###.", "...#.", "...#.", "...#.", "...#.", "...#.", ".###."], // ]
    ["..#..", ".#.#.", "#...#", ".....", ".....", ".....", "....."], // ^
    [".....", ".....", ".....", ".....", ".....", ".....", "#####"], // _
    [".#...", "..#..", "...#.", ".....", ".....", ".....", "....."], // `
    [".....", ".....", ".###.", "....#", ".####", "#...#", ".####"], // a
    ["#....", "#....", "#.##.", "##..#", "#...#", "#...#", "####."], // b
    [".....", ".....", ".###.", "#....", "#....", "#...#", ".###."], // c
    ["....#", "....#", ".##.#", "#..##", "#...#", "#...#", ".####"], // d
    [".....", ".....", ".###.", "#...#", "#####", "#....", ".###."], // e
    ["..##.", ".#..#", ".#...", "###..", ".#...", ".#...", ".#..."], // f
    [".....", ".....", ".####", "#...#", ".####", "....#", ".###."], // g
    ["#....", "#....", "#.##.", "##..#", "#...#", "#...#", "#...#"], // h
    ["..#..", ".....", ".##..", "..#..", "..#..", "..#..", ".###."], // i
    ["...#.", ".....", "..##.", "...#.", "...#.", "#..#.", ".##.."], // j
    ["#....", "#....", "#..#.", "#.#..", "##...", "#.#..", "#..#."], // k
    [".##..", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."], // l
    [".....", ".....", "##.#.", "#.#.#", "#.#.#", "#...#", "#...#"], // m
    [".....", ".....", "#.##.", "##..#", "#...#", "#...#", "#...#"], // n
    [".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###."], // o
    [".....", ".....", "####.", "#...#", "####.", "#....", "#...."], // p
    [".....", ".....", ".##.#", "#..##", ".####", "....#", "....#"], // q
    [".....", ".....", "#.##.", "##..#", "#....", "#....", "#...."], // r
    [".....", ".....", ".###.", "#....", ".###.", "....#", "####."], // s
    [".#...", ".#...", "###..", ".#...", ".#...", ".#..#", "..##."], // t
    [".....", ".....", "#...#", "#...#", "#...#", "#..##", ".##.#"], // u
    [".....", ".....", "#...#", "#...#", "#...#", ".#.#.", "..#.."], // v
    [".....", ".....", "#...#", "#...#", "#.#.#", "#.#.#", ".#.#."], // w
    [".....", ".....", "#...#", ".#.#.", "..#..", ".#.#.", "#...#"], // x
    [".....", ".....", "#...#", "#...#", ".####", "....#", ".###."], // y
    [".....", ".....", "#####", "...#.", "..#..", ".#...", "#####"], // z
    ["...#.", "..#..", "..#..", ".#...", "..#..", "..#..", "...#."], // {
    ["..#..", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."], // |
    [".#...", "..#..", "..#..", "...#.", "..#..", "..#..", ".#..."], // }
    [".....", ".....", ".#...", "#.#.#", "...#.", ".....", "....."], // ~
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_font_covers_printable_ascii() {
        assert_eq!(FONT.len(), 95);
        for (i, glyph) in FONT.iter().enumerate() {
            for row in glyph {
                assert_eq!(row.len(), 5, "glyph {} row {row:?}", i + 32);
                assert!(
                    row.bytes().all(|b| b == b'#' || b == b'.'),
                    "glyph {}",
                    i + 32
                );
            }
        }
        let atlas = rasterize_builtin();
        assert_eq!(atlas.pixels.len(), 12 * 95 * 16);
        // 'A' (glyph 33) has ink; space (glyph 0) has none.
        let stride = 12 * 95;
        let ink = |glyph: usize| {
            (0..16)
                .flat_map(|y| (0..12).map(move |x| (x, y)))
                .filter(|&(x, y)| atlas.pixels[y * stride + glyph * 12 + x] > 0)
                .count()
        };
        assert!(ink(33) > 20);
        assert_eq!(ink(0), 0);
    }

    #[test]
    fn a_ttf_rasterises_when_present() {
        let path = crate::workspace_root_from(env!("CARGO_MANIFEST_DIR"))
            .join("assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf");
        let Ok(bytes) = std::fs::read(&path) else {
            eprintln!("font not present, skipping: {}", path.display());
            return;
        };
        let atlas = rasterize_ttf(&bytes, 14.0).expect("rasterised");
        assert!(
            atlas.cell_w >= 7 && atlas.cell_w <= 12,
            "cell width {}",
            atlas.cell_w
        );
        assert!(
            atlas.cell_h >= 14 && atlas.cell_h <= 24,
            "cell height {}",
            atlas.cell_h
        );
        let stride = atlas.cell_w as usize * 95;
        let ink: usize = atlas.pixels.iter().filter(|&&p| p > 0).count();
        assert!(ink > 2000, "ink {ink}");
        assert!(atlas.pixels[..stride].len() == stride);
    }

    #[test]
    fn canvas_text_and_bars_pack_as_the_shader_expects() {
        let mut canvas = Canvas::new();
        canvas.reset(10, 2);
        canvas.panel(0, 0, 10, 1, Color::Panel);
        let next = canvas.text(1, 0, "Hi", Color::White);
        assert_eq!(next, 3);
        let cell = canvas.cells[1];
        assert_eq!(cell & 0xFF, u32::from(b'H'));
        assert_eq!((cell >> 8) & 0xF, Color::White as u32);
        assert_eq!((cell >> 12) & 0xF, Color::Panel as u32);
        canvas.bar(0, 1, 4, 0.5, Color::Cyan);
        let fills: Vec<u32> = (0..4)
            .map(|c| (canvas.cells[10 + c] >> 16) & 0xFF)
            .collect();
        assert_eq!(fills, vec![255, 255, 0, 0]);
        assert_eq!(canvas.cells[10] & 0xFF, BAR_GLYPH);
    }
}

//! Local exposure (D-067, #275; `shaders/local_exposure_grid.slang`, `shaders/local_exposure.slang`):
//! a dark hall beside a sunlit doorway lifted, the doorway lowered a little, each by a smooth
//! per-pixel exposure gain applied just before the display transform.
//!
//! **The method** (Durand & Dorsey 2002's base and detail, on Chen, Paris & Durand 2007's
//! bilateral grid, blended as Patry 2021 shipped it in Ghost of Tsushima). This frame's resolved
//! HDR image (TAA's history or DLAA's output) is splatted into a 64 × 36 × 64 grid of log2
//! luminance, blurred, and the cells' mean log luminance blurred by a wide Gaussian. Each pixel
//! reads its base B, 40 % from the grid at its own position and luminance and 60 % from the
//! Gaussian, and takes the gain `2^((c − 1)(B − M))`, c the shadow contrast under the middle grey
//! M the automatic exposure keys the scene on ([`middle_grey`]), the highlight contrast over it,
//! clamped to [`MAX_LIFT`] and [`MAX_DARKEN`], none in near-black. Detail passes through: the
//! gain depends on the base alone.
//!
//! **Within one frame.** The grid is built from this frame's image and read by this frame's
//! display pass, in transient images that never outlive the frame: no grid history, so nothing
//! of one frame modifies the next (D-067: clear of STMicroelectronics' US 11,756,172 B2, whose
//! claims use an earlier frame's low-resolution log-luminance image on a later frame). Never add
//! temporal reuse of the grid. Two more rules keep it clear of that family (US and EP 3872750 B1,
//! `research/lighting-patents.md`): no strength as a mix of the gain (`(gain − 1) · s + 1` is the
//! claims' formula; a strength scales the contrast c, in the log domain), and no gain map
//! stored: each pixel computes its gain in the display pass.
//!
//! **Where.** After TAA or DLAA and outside their history (the history is rescaled by one ratio
//! of exposures, DLSS takes one exposure value), after bloom is mixed in, before the tone curve.
//! The histogram keeps metering the image before it: a meter that saw the gain would find a key
//! already pulled towards M.
//!
//! [`CpuGrid`] and [`gain_ev`] mirror the shaders and pin their behaviour in tests.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, ImageAccess, ImageHandle, PassBuilder, Pipeline,
    Resources, Result, ShaderCompiler, ShaderStage, TransientDesc, vk,
};

/// Cells across the image (must match `local_exposure.slang`).
pub const CELLS_X: usize = 64;
/// Cells down the image.
pub const CELLS_Y: usize = 36;
/// Slices of log2 luminance.
pub const SLICES: usize = 64;
/// Slices across the atlas the grid's slices are laid out in (8 × 8 tiles of 64 × 36).
const TILES_X: usize = 8;
/// Log2 of the pre-exposed luminance at the bottom of the grid's range (the histogram's).
pub const MIN_LOG2: f32 = -16.0;
/// Log2 of the pre-exposed luminance at its top.
pub const MAX_LOG2: f32 = 8.0;
/// EV a slice spans: 0.375.
const SLICE: f32 = (MAX_LOG2 - MIN_LOG2) / SLICES as f32;
/// The most the gain lifts, EV.
pub const MAX_LIFT: f32 = 3.0;
/// The most it darkens, EV.
pub const MAX_DARKEN: f32 = 2.0;
/// No gain on pixels this many EV under M, all of it [`DARK_FADE`] above.
pub const DARK_FLOOR: f32 = 12.0;
/// EV over which the gain fades in above the dark floor.
pub const DARK_FADE: f32 = 2.0;
/// A grid read of less weight (the blurred share of a cell's pixels in that luminance) fades to
/// the Gaussian's base.
const MIN_WEIGHT: f32 = 0.01;
/// The grid's blur across cells: a Gaussian of σ = 1 cell, three cells each side.
const GRID_GAUSSIAN: [f32; 4] = [1.0, 0.606_530_66, 0.135_335_28, 0.011_108_997];
/// The cells' means' Gaussian: radius 13 cells of 64 (Patry), σ a third of it.
const MEANS_RADIUS: usize = 13;
/// The largest cell the grid pass's fixed-point sums hold: 128 × 128 pixels (a frame up to
/// 8192 × 4608).
const MAX_CELL: u32 = 128;
const GROUP: u32 = 8;

/// Local exposure's settings (D-067's defaults).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalExposureSettings {
    /// On at all: off by default until the owner has seen it (D-067).
    pub enabled: bool,
    /// The base's contrast under middle grey (1: unchanged; 0.7 lifts a hall 5 EV under middle
    /// grey by 1.5 EV).
    pub shadow_contrast: f32,
    /// The base's contrast over middle grey (0.85 lowers a doorway 3 EV over by 0.45 EV).
    pub highlight_contrast: f32,
    /// The Gaussian's share of the base, the grid's the rest (Patry: 60 % Gaussian; Epic:
    /// 0.4–0.6): it hides the grid's ringing on smooth gradients, at a soft, wide halo.
    pub blend: f32,
    /// EV added to the middle grey the base is pulled towards (Epic's control): negative darker.
    pub middle_grey_bias: f32,
    /// Show the gain in false colours: blue where it lifts, red where it darkens.
    pub view: bool,
}

impl Default for LocalExposureSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            shadow_contrast: 0.7,
            highlight_contrast: 0.85,
            blend: 0.6,
            middle_grey_bias: 0.0,
            view: false,
        }
    }
}

impl LocalExposureSettings {
    /// The F1 overlay's line.
    pub fn line(&self, middle: f32) -> String {
        if !self.enabled {
            return "local exposure off".to_owned();
        }
        format!(
            "local exposure{}: contrast {:.2} under / {:.2} over middle grey ({:+.2} EV), blend {:.2}",
            if self.view { " (gain view)" } else { "" },
            self.shadow_contrast,
            self.highlight_contrast,
            middle,
            self.blend,
        )
    }
}

/// M: log2 of the pre-exposed luminance the automatic exposure keys the scene on (D-022: the
/// key lands at `exposure · L = 2^compensation / 9.6`), less the stops the night is shown darker
/// by (D-046: without them local exposure would lift the night back up).
pub fn middle_grey(compensation: f32, night_offset: f32) -> f32 {
    compensation - night_offset - 9.6_f32.log2()
}

/// A pixel's log2 luminance, clamped to the grid's range (mirror of `local_log_luminance`).
pub fn log_luminance(luminance: f32) -> f32 {
    luminance
        .max(MIN_LOG2.exp2())
        .log2()
        .clamp(MIN_LOG2, MAX_LOG2)
}

/// The slice coordinate of a log luminance: slice i's centre at i (mirror of `local_slice`).
fn slice(l: f32) -> f32 {
    ((l - MIN_LOG2) / SLICE - 0.5).clamp(0.0, (SLICES - 1) as f32)
}

/// The gain in EV of a pixel of log luminance `l` whose base is `base`, middle grey `middle`
/// (mirror of `local_gain_ev`).
pub fn gain_ev(settings: &LocalExposureSettings, middle: f32, l: f32, base: f32) -> f32 {
    let c = if base < middle {
        settings.shadow_contrast
    } else {
        settings.highlight_contrast
    };
    let ev = ((c - 1.0) * (base - middle)).clamp(-MAX_DARKEN, MAX_LIFT);
    ev * ((l - (middle - DARK_FLOOR)) / DARK_FADE).clamp(0.0, 1.0)
}

/// The CPU mirror of the grid passes and of the display pass's read (tests, tools): the grid's
/// cells as `(Σ w·l, Σ w)` per pixel, blurred, and the cells' blurred means.
pub struct CpuGrid {
    /// Cell (x, y) of slice s at `(s * CELLS_Y + y) * CELLS_X + x`.
    cells: Vec<[f32; 2]>,
    /// The cells' means, blurred, at `y * CELLS_X + x`.
    means: Vec<f32>,
}

fn cell_index(x: usize, y: usize, s: usize) -> usize {
    (s * CELLS_Y + y) * CELLS_X + x
}

impl CpuGrid {
    /// The grid of an image's pre-exposed luminance (`width × height`, rows from the top), as
    /// the grid and blur passes build it.
    pub fn build(luminance: &[f32], width: usize, height: usize) -> Self {
        assert_eq!(luminance.len(), width * height);
        let mut cells = vec![[0.0_f32; 2]; CELLS_X * CELLS_Y * SLICES];
        let mut means = vec![0.0_f32; CELLS_X * CELLS_Y];
        for cy in 0..CELLS_Y {
            let (y0, y1) = (cy * height / CELLS_Y, (cy + 1) * height / CELLS_Y);
            for cx in 0..CELLS_X {
                let (x0, x1) = (cx * width / CELLS_X, (cx + 1) * width / CELLS_X);
                let pixels = ((x1 - x0) * (y1 - y0)).max(1) as f32;
                let mut column = [[0.0_f32; 2]; SLICES];
                let mut sum = 0.0;
                for y in y0..y1 {
                    for &value in &luminance[y * width + x0..y * width + x1] {
                        let l = log_luminance(value);
                        let z = slice(l);
                        let z0 = z as usize;
                        let f = z - z0 as f32;
                        column[z0][0] += (1.0 - f) * l;
                        column[z0][1] += 1.0 - f;
                        if f > 0.0 {
                            column[z0 + 1][0] += f * l;
                            column[z0 + 1][1] += f;
                        }
                        sum += l;
                    }
                }
                for (s, [wl, w]) in column.into_iter().enumerate() {
                    cells[cell_index(cx, cy, s)] = [wl / pixels, w / pixels];
                }
                means[cy * CELLS_X + cx] = sum / pixels;
            }
        }
        let blurred = blur_cells(&cells, |at, d| {
            let x = at.0 as isize + d;
            (0..CELLS_X as isize)
                .contains(&x)
                .then(|| cell_index(x as usize, at.1, at.2))
        });
        let cells = blur_yz(&blurred);
        Self {
            cells,
            means: blur_means(&means),
        }
    }

    /// The bilinear read of slice `s` at `uv`, clamped to the slice's tile (mirror of `grid_tap`).
    fn tap(&self, u: f32, v: f32, s: usize) -> [f32; 2] {
        let bilinear = |at: f32, cells: usize| {
            let t = (at * cells as f32).clamp(0.5, cells as f32 - 0.5) - 0.5;
            let i = (t.floor() as usize).min(cells - 1);
            (i, (i + 1).min(cells - 1), t - i as f32)
        };
        let (x0, x1, fx) = bilinear(u, CELLS_X);
        let (y0, y1, fy) = bilinear(v, CELLS_Y);
        let c = |x, y| self.cells[cell_index(x, y, s)];
        let lerp =
            |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        lerp(
            lerp(c(x0, y0), c(x1, y0), fx),
            lerp(c(x0, y1), c(x1, y1), fx),
            fy,
        )
    }

    /// The cells' blurred means at `uv` through the cubic B-spline of four bilinear taps (mirror
    /// of `blurred_mean`).
    pub fn blurred(&self, u: f32, v: f32) -> f32 {
        let bilinear = |x: f32, y: f32| {
            let clamped = |t: f32, n: usize| {
                let t = t - 0.5;
                let i = t.floor();
                let f = t - i;
                let at = |k: f32| (k.max(0.0) as usize).min(n - 1);
                (at(i), at(i + 1.0), f)
            };
            let (x0, x1, fx) = clamped(x * CELLS_X as f32, CELLS_X);
            let (y0, y1, fy) = clamped(y * CELLS_Y as f32, CELLS_Y);
            let m = |x: usize, y: usize| self.means[y * CELLS_X + x];
            let top = m(x0, y0) + (m(x1, y0) - m(x0, y0)) * fx;
            let bottom = m(x0, y1) + (m(x1, y1) - m(x0, y1)) * fx;
            top + (bottom - top) * fy
        };
        let spline = |at: f32, cells: usize| {
            let t = at * cells as f32 - 0.5;
            let i = t.floor();
            let f = t - i;
            let (f2, f3) = (f * f, f * f * f);
            let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
            let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
            let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
            let w3 = f3 / 6.0;
            let (g0, g1) = (w0 + w1, w2 + w3);
            let h0 = (i - 0.5 + w1 / g0) / cells as f32;
            let h1 = (i + 1.5 + w3 / g1) / cells as f32;
            (g0, g1, h0, h1)
        };
        let (gx0, gx1, hx0, hx1) = spline(u, CELLS_X);
        let (gy0, gy1, hy0, hy1) = spline(v, CELLS_Y);
        gy0 * (gx0 * bilinear(hx0, hy0) + gx1 * bilinear(hx1, hy0))
            + gy1 * (gx0 * bilinear(hx0, hy1) + gx1 * bilinear(hx1, hy1))
    }

    /// The base log luminance of a pixel of log luminance `l` at `uv` (mirror of `local_base`).
    pub fn base(&self, u: f32, v: f32, l: f32, blend: f32) -> f32 {
        let blurred = self.blurred(u, v);
        let z = slice(l);
        let z0 = z as usize;
        let z1 = (z0 + 1).min(SLICES - 1);
        let (a, b, f) = (self.tap(u, v, z0), self.tap(u, v, z1), z - z0 as f32);
        let g = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
        let confidence = (g[1] / MIN_WEIGHT).clamp(0.0, 1.0);
        let bilateral = blurred + (g[0] / g[1].max(1e-6) - blurred) * confidence;
        bilateral + (blurred - bilateral) * blend
    }

    /// The gain in EV at pixel (x, y) of an image `width × height` whose pre-exposed luminance
    /// there is `luminance` (the display pass's read and gain).
    #[allow(clippy::too_many_arguments)]
    pub fn gain_at(
        &self,
        settings: &LocalExposureSettings,
        middle: f32,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
        luminance: f32,
    ) -> f32 {
        let (u, v) = (
            (x as f32 + 0.5) / width as f32,
            (y as f32 + 0.5) / height as f32,
        );
        let l = log_luminance(luminance);
        gain_ev(settings, middle, l, self.base(u, v, l, settings.blend))
    }
}

/// One pass of the grid's Gaussian along an axis: `neighbour(cell, d)` the index d cells away,
/// `None` off the image; renormalised at the edges.
fn blur_cells(
    cells: &[[f32; 2]],
    neighbour: impl Fn((usize, usize, usize), isize) -> Option<usize>,
) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0; 2]; cells.len()];
    for s in 0..SLICES {
        for y in 0..CELLS_Y {
            for x in 0..CELLS_X {
                let (mut sum, mut weights) = ([0.0_f32; 2], 0.0);
                for d in -3_isize..=3 {
                    if let Some(i) = neighbour((x, y, s), d) {
                        let w = GRID_GAUSSIAN[d.unsigned_abs()];
                        sum[0] += w * cells[i][0];
                        sum[1] += w * cells[i][1];
                        weights += w;
                    }
                }
                out[cell_index(x, y, s)] = [sum[0] / weights, sum[1] / weights];
            }
        }
    }
    out
}

/// The grid's blur in Y and its [1 2 1] / 4 tent across the slices, in one pass (as the shader).
fn blur_yz(cells: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0; 2]; cells.len()];
    for s in 0..SLICES {
        for y in 0..CELLS_Y {
            for x in 0..CELLS_X {
                let (mut sum, mut weights) = ([0.0_f32; 2], 0.0);
                for dz in -1_isize..=1 {
                    let z = s as isize + dz;
                    if !(0..SLICES as isize).contains(&z) {
                        continue;
                    }
                    let wz = if dz == 0 { 2.0 } else { 1.0 };
                    for d in -3_isize..=3 {
                        let at = y as isize + d;
                        if !(0..CELLS_Y as isize).contains(&at) {
                            continue;
                        }
                        let w = wz * GRID_GAUSSIAN[d.unsigned_abs()];
                        let c = cells[cell_index(x, at as usize, z as usize)];
                        sum[0] += w * c[0];
                        sum[1] += w * c[1];
                        weights += w;
                    }
                }
                out[cell_index(x, y, s)] = [sum[0] / weights, sum[1] / weights];
            }
        }
    }
    out
}

/// The cells' means through the wide Gaussian, X then Y, renormalised at the edges (mirror of
/// `means_main`).
fn blur_means(means: &[f32]) -> Vec<f32> {
    let sigma = MEANS_RADIUS as f32 / 3.0;
    let gaussian: Vec<f32> = (0..=MEANS_RADIUS)
        .map(|d| (-0.5 * (d * d) as f32 / (sigma * sigma)).exp())
        .collect();
    let radius = MEANS_RADIUS as isize;
    let pass = |input: &[f32], along_x: bool| {
        let mut out = vec![0.0; input.len()];
        for y in 0..CELLS_Y {
            for x in 0..CELLS_X {
                let (mut sum, mut weights) = (0.0, 0.0);
                for d in -radius..=radius {
                    let (at, n) = if along_x {
                        (x as isize + d, CELLS_X)
                    } else {
                        (y as isize + d, CELLS_Y)
                    };
                    if !(0..n as isize).contains(&at) {
                        continue;
                    }
                    let i = if along_x {
                        y * CELLS_X + at as usize
                    } else {
                        at as usize * CELLS_X + x
                    };
                    let w = gaussian[d.unsigned_abs()];
                    sum += w * input[i];
                    weights += w;
                }
                out[y * CELLS_X + x] = sum / weights;
            }
        }
        out
    };
    pass(&pass(means, true), false)
}

/// Mirrors `LocalExposure` in `local_exposure.slang`: what the display passes read.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct LocalExposurePush {
    grid: u32,
    means: u32,
    middle: f32,
    shadow: f32,
    highlight: f32,
    blend: f32,
    view: u32,
    pad: u32,
}

impl LocalExposurePush {
    /// No local exposure: the display passes show the image as before.
    pub const OFF: Self = Self {
        grid: u32::MAX,
        means: u32::MAX,
        middle: 0.0,
        shadow: 1.0,
        highlight: 1.0,
        blend: 0.0,
        view: 0,
        pad: 0,
    };
}

/// What a display pass reads of this frame's grid ([`LocalExposure::build`]): the images, which
/// it declares, and the settings.
#[derive(Clone, Copy, Debug)]
pub struct LocalExposureRead {
    grid: ImageHandle,
    means: ImageHandle,
    middle: f32,
    settings: LocalExposureSettings,
}

impl LocalExposureRead {
    /// Declares the grid and the means as sampled by `pass`'s fragment shader.
    pub fn declare<'b, 'f>(&self, pass: PassBuilder<'b, 'f>) -> PassBuilder<'b, 'f> {
        let sampled = ImageAccess::Sampled(vk::PipelineStageFlags2::FRAGMENT_SHADER);
        pass.image(self.grid, sampled).image(self.means, sampled)
    }

    /// The push block of a pass that declared the read.
    pub fn push(&self, resources: &Resources<'_>) -> LocalExposurePush {
        LocalExposurePush {
            grid: resources.sampled(self.grid).0,
            means: resources.sampled(self.means).0,
            middle: self.middle + self.settings.middle_grey_bias,
            shadow: self.settings.shadow_contrast,
            highlight: self.settings.highlight_contrast,
            blend: self.settings.blend,
            view: u32::from(self.settings.view),
            pad: 0,
        }
    }
}

/// Mirrors `Push` in `local_exposure_grid.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridPush {
    src: u32,
    dst: u32,
    means: u32,
    width: u32,
    height: u32,
    pad: [u32; 3],
}

/// The grid's passes.
pub struct LocalExposure {
    grid: Pipeline,
    blur_x: Pipeline,
    blur_yz: Pipeline,
    means: Pipeline,
}

impl LocalExposure {
    /// Compiles the passes.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("local_exposure_grid.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: std::mem::size_of::<GridPush>() as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        Ok(Self {
            grid: compute("grid_main", "local exposure grid")?,
            blur_x: compute("blur_x_main", "local exposure blur x")?,
            blur_yz: compute("blur_yz_main", "local exposure blur yz")?,
            means: compute("means_main", "local exposure means")?,
        })
    }

    /// Declares the passes "post/local exposure grid" and "post/local exposure blur" over
    /// `image`, this frame's resolved HDR image of `extent`, and returns what the display pass
    /// reads with `settings` around the middle grey `middle` ([`middle_grey`]). Everything it
    /// makes is a transient of this frame.
    pub fn build<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        image: ImageHandle,
        extent: vk::Extent2D,
        settings: LocalExposureSettings,
        middle: f32,
    ) -> LocalExposureRead {
        debug_assert!(
            extent.width.div_ceil(CELLS_X as u32) <= MAX_CELL
                && extent.height.div_ceil(CELLS_Y as u32) <= MAX_CELL,
            "{extent:?} is too large for the grid's sums"
        );
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let transient = |graph: &mut FrameGraph<'f>, name, width, height| {
            graph.transient(TransientDesc {
                name,
                width: width as u32,
                height: height as u32,
                format: vk::Format::R16G16_SFLOAT,
                usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
            })
        };
        let (atlas_w, atlas_h) = (CELLS_X * TILES_X, CELLS_Y * SLICES / TILES_X);
        let grid = transient(graph, "local exposure grid", atlas_w, atlas_h);
        let across = transient(graph, "local exposure grid across", atlas_w, atlas_h);
        let means = transient(graph, "local exposure means", CELLS_X, CELLS_Y);
        let blurred = transient(graph, "local exposure means blurred", CELLS_X, CELLS_Y);
        let (pipeline_grid, pipeline_x, pipeline_yz, pipeline_means) =
            (&self.grid, &self.blur_x, &self.blur_yz, &self.means);
        let atlas_groups = (
            (atlas_w as u32).div_ceil(GROUP),
            (atlas_h as u32).div_ceil(GROUP),
        );
        graph
            .pass("post/local exposure grid")
            .image(image, ImageAccess::Sampled(compute))
            .image(grid, ImageAccess::StorageWrite(compute))
            .image(means, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline_grid);
                commands.push_constants(
                    pipeline_grid,
                    &GridPush {
                        src: resources.sampled(image).0,
                        dst: resources.storage(grid, 0).0,
                        means: resources.storage(means, 0).0,
                        width: extent.width,
                        height: extent.height,
                        pad: [0; 3],
                    },
                );
                commands.dispatch(CELLS_X as u32, CELLS_Y as u32, 1);
                Ok(())
            });
        // Across the cells, and the means' Gaussian beside it (independent: one pass).
        graph
            .pass("post/local exposure blur")
            .image(grid, ImageAccess::Sampled(compute))
            .image(across, ImageAccess::StorageWrite(compute))
            .image(means, ImageAccess::Sampled(compute))
            .image(blurred, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline_x);
                commands.push_constants(
                    pipeline_x,
                    &GridPush {
                        src: resources.sampled(grid).0,
                        dst: resources.storage(across, 0).0,
                        means: 0,
                        width: 0,
                        height: 0,
                        pad: [0; 3],
                    },
                );
                commands.dispatch(atlas_groups.0, atlas_groups.1, 1);
                commands.bind_pipeline(pipeline_means);
                commands.push_constants(
                    pipeline_means,
                    &GridPush {
                        src: resources.sampled(means).0,
                        dst: resources.storage(blurred, 0).0,
                        means: 0,
                        width: 0,
                        height: 0,
                        pad: [0; 3],
                    },
                );
                commands.dispatch(1, 1, 1);
                Ok(())
            });
        // Down the cells and across the slices, back into the grid.
        graph
            .pass("post/local exposure blur")
            .image(across, ImageAccess::Sampled(compute))
            .image(grid, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline_yz);
                commands.push_constants(
                    pipeline_yz,
                    &GridPush {
                        src: resources.sampled(across).0,
                        dst: resources.storage(grid, 0).0,
                        means: 0,
                        width: 0,
                        height: 0,
                        pad: [0; 3],
                    },
                );
                commands.dispatch(atlas_groups.0, atlas_groups.1, 1);
                Ok(())
            });
        LocalExposureRead {
            grid,
            means: blurred,
            middle,
            settings,
        }
    }
}

/// The push block of a display pass: `read`'s, or [`LocalExposurePush::OFF`].
pub fn push_of(read: Option<LocalExposureRead>, resources: &Resources<'_>) -> LocalExposurePush {
    read.map_or(LocalExposurePush::OFF, |r| r.push(resources))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exposure::{AutoExposure, LuminanceHistogram};

    const M: f32 = -2.263_034_4; // middle_grey(1, 0): log2(2 / 9.6)

    fn defaults() -> LocalExposureSettings {
        LocalExposureSettings {
            enabled: true,
            ..Default::default()
        }
    }

    /// A `width × height` image whose log2 luminance is `l(x, y)`.
    fn image(width: usize, height: usize, l: impl Fn(usize, usize) -> f32) -> Vec<f32> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| l(x, y).exp2())
            .collect()
    }

    #[test]
    fn the_push_block_matches_the_shader() {
        assert_eq!(std::mem::size_of::<LocalExposurePush>(), 32);
        assert_eq!(std::mem::size_of::<GridPush>(), 32);
        assert!((middle_grey(1.0, 0.0) - M).abs() < 1e-5);
    }

    #[test]
    fn the_metered_key_lands_on_middle_grey() {
        // A uniform scene metered by D-022's automatic exposure, compensated a stop: its pixels
        // land at M, where local exposure leaves them alone.
        for luminance in [0.3_f32, 40.0, 5000.0] {
            let mut auto = AutoExposure::new(10.0);
            auto.compensation = 1.0;
            let h = LuminanceHistogram::from_samples([(luminance, 1000)], auto.exposure());
            auto.update(Some(&h), 1.0 / 60.0);
            let exposed = (luminance * auto.exposure()).log2();
            let middle = middle_grey(auto.compensation, auto.night_offset);
            assert!(
                (exposed - middle).abs() < 0.05,
                "{exposed} against {middle}"
            );
            assert_eq!(gain_ev(&defaults(), middle, middle, middle), 0.0);
        }
    }

    #[test]
    fn contrast_one_gives_no_gain() {
        let neutral = LocalExposureSettings {
            shadow_contrast: 1.0,
            highlight_contrast: 1.0,
            ..defaults()
        };
        for base in [-15.0, -8.0, M - 0.01, M, M + 0.01, 0.0, 7.5] {
            for l in [-16.0, -10.0, base, 4.0] {
                assert_eq!(gain_ev(&neutral, M, l, base), 0.0, "{base} {l}");
            }
        }
    }

    #[test]
    fn the_gain_lifts_shadows_lowers_highlights_and_is_clamped() {
        let s = defaults();
        // A hall 5 EV under middle grey: lifted 1.5 EV; a doorway 3 EV over: lowered 0.45 EV.
        assert!((gain_ev(&s, M, M - 5.0, M - 5.0) - 1.5).abs() < 1e-5);
        assert!((gain_ev(&s, M, M + 3.0, M + 3.0) + 0.45).abs() < 1e-5);
        // Clamped to +3 EV and −2 EV.
        assert_eq!(gain_ev(&s, M, M - 9.9, M - 11.0), MAX_LIFT);
        assert_eq!(gain_ev(&s, M, M + 8.0, M + 20.0), -MAX_DARKEN);
        // None in near-black: below M − 12 EV, faded in over 2 EV.
        assert_eq!(gain_ev(&s, M, M - 12.5, M - 5.0), 0.0);
        let half = gain_ev(&s, M, M - 11.0, M - 5.0);
        assert!((half - 0.75).abs() < 1e-5, "{half}");
        // Continuous at middle grey.
        assert!(gain_ev(&s, M, M, M - 1e-4).abs() < 1e-4);
        assert!(gain_ev(&s, M, M, M + 1e-4).abs() < 1e-4);
    }

    #[test]
    fn two_flat_regions_take_their_own_base() {
        // A hall 5 EV under middle grey beside a street 3 EV over: away from the edge (beyond the
        // Gaussian's 13 cells) each region's base is its own luminance, its gain (1 − c)(M − B).
        let (w, h) = (640, 360);
        let (hall, street) = (M - 5.0, M + 3.0);
        let lum = image(w, h, |x, _| if x < w / 2 { hall } else { street });
        let grid = CpuGrid::build(&lum, w, h);
        let s = defaults();
        let cell = w / CELLS_X;
        for x in (0..w).step_by(5) {
            let cells_from_edge = (x as isize - (w / 2) as isize).unsigned_abs() / cell;
            for y in [5, h / 2, h - 5] {
                let l = if x < w / 2 { hall } else { street };
                let base = grid.base(
                    (x as f32 + 0.5) / w as f32,
                    (y as f32 + 0.5) / h as f32,
                    l,
                    s.blend,
                );
                if cells_from_edge > MEANS_RADIUS + 1 {
                    assert!((base - l).abs() < 0.01, "x {x}: base {base} for {l}");
                    let want = gain_ev(&s, M, l, l);
                    let got = grid.gain_at(&s, M, x, y, w, h, l.exp2());
                    assert!((got - want).abs() < 0.01, "x {x}: gain {got} for {want}");
                }
            }
        }
    }

    #[test]
    fn a_step_edge_on_the_grid_alone_has_no_gain_gradient_beyond_a_cell() {
        // The bilateral-only path (blend 0): a pixel reads the slices of its own luminance, so the
        // street does not darken the hall beside it (no halo) beyond a cell from the edge.
        let (w, h) = (640, 360);
        let (hall, street) = (M - 5.0, M + 3.0);
        let edge = w / 2 + 3; // not on a cell's border
        let lum = image(w, h, |x, _| if x < edge { hall } else { street });
        let grid = CpuGrid::build(&lum, w, h);
        let s = LocalExposureSettings {
            blend: 0.0,
            ..defaults()
        };
        let cell = w / CELLS_X;
        let (want_hall, want_street) = (gain_ev(&s, M, hall, hall), gain_ev(&s, M, street, street));
        for x in 0..w {
            if x.abs_diff(edge) <= cell {
                continue;
            }
            let (l, want) = if x < edge {
                (hall, want_hall)
            } else {
                (street, want_street)
            };
            let got = grid.gain_at(&s, M, x, h / 2, w, h, l.exp2());
            assert!((got - want).abs() < 0.01, "x {x}: {got} against {want}");
        }
    }

    #[test]
    fn a_ramp_and_a_step_stay_monotonic() {
        // A ramp of 10 EV across the image with a 3 EV step in its middle: the shown luminance
        // (l + gain) still rises everywhere along it, with the defaults and on the grid alone (no
        // gradient reversal).
        let (w, h) = (640, 360);
        let l =
            |x: usize| M - 8.0 + 10.0 * x as f32 / w as f32 + if x >= w / 2 { 3.0 } else { 0.0 };
        let lum = image(w, h, |x, _| l(x));
        let grid = CpuGrid::build(&lum, w, h);
        for blend in [0.6, 0.0, 1.0] {
            let s = LocalExposureSettings {
                blend,
                ..defaults()
            };
            for y in [0, h / 3, h / 2, h - 1] {
                let shown: Vec<f32> = (0..w)
                    .map(|x| l(x) + grid.gain_at(&s, M, x, y, w, h, l(x).exp2()))
                    .collect();
                for x in 1..w {
                    assert!(
                        shown[x] > shown[x - 1],
                        "blend {blend}, y {y}, x {x}: {} after {}",
                        shown[x],
                        shown[x - 1]
                    );
                }
            }
        }
    }

    #[test]
    fn the_blurred_means_follow_a_flat_image_and_its_slices_their_centres() {
        let (w, h) = (128, 72);
        let grid = CpuGrid::build(&image(w, h, |_, _| -3.0), w, h);
        for (u, v) in [(0.0, 0.0), (0.5, 0.5), (0.99, 0.2), (1.0, 1.0)] {
            assert!((grid.blurred(u, v) + 3.0).abs() < 1e-4);
            assert!((grid.base(u, v, -3.0, 0.0) + 3.0).abs() < 1e-4);
        }
        for s in 0..SLICES {
            let centre = MIN_LOG2 + (s as f32 + 0.5) * SLICE;
            assert!((slice(centre) - s as f32).abs() < 1e-4);
        }
        assert_eq!(log_luminance(0.0), MIN_LOG2);
        assert_eq!(log_luminance(1e9), MAX_LOG2);
    }
}

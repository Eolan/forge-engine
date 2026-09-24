//! Physical camera exposure and automatic exposure from a luminance histogram
//! (`shaders/exposure.slang`).
//!
//! **Units.** Lights are given in physical units (the sun in lux) and surfaces return
//! luminance in cd/m². The frame is drawn **pre-exposed** (Lagarde & de Rousiers, "Moving
//! Frostbite to PBR", 2014): every shader multiplies its luminance by the frame's exposure,
//! so the HDR target holds values near 1 whether the scene is starlight or noon, and fp16 is
//! enough. The exposure comes from an exposure value at ISO 100 (EV100) through the
//! saturation-based sensitivity: the luminance `1.2 · 2^EV100` maps to 1.
//!
//! **Metering** (Narkowicz, "Automatic Exposure", 2016). A compute pass bins the luminance
//! of every pixel on a log2 scale; the CPU reads the bins two frames later, drops the black
//! bin, keeps the samples between two fractions of the sorted rest, takes their log-average
//! as the scene key and adapts EV100 towards the value that meters that key as middle grey,
//! with separate speeds for brightening and darkening and a clamp to a range of EV100.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};

/// Bins of the histogram (must match `exposure.slang`).
pub const HISTOGRAM_BINS: usize = 256;
/// Log2 of the pre-exposed luminance at the bottom of bin 1; bin 0 holds everything below.
const MIN_LOG2: f32 = -16.0;
/// Log2 of the pre-exposed luminance at the top of the metered range; the last bin also
/// holds everything above.
const MAX_LOG2: f32 = 8.0;

/// Exposure multiplier for an exposure value at ISO 100: the luminance `1.2 · 2^EV100`
/// (cd/m²) saturates the sensor and maps to 1.
pub fn exposure_from_ev100(ev100: f32) -> f32 {
    1.0 / (1.2 * ev100.exp2())
}

/// The EV100 a reflected-light meter (calibration constant K = 12.5) reads for an average
/// scene luminance in cd/m².
pub fn ev100_from_luminance(luminance: f32) -> f32 {
    (luminance.max(1e-12) * 100.0 / 12.5).log2()
}

/// Luminance of a white Lambertian surface facing a light of `illuminance` lux, in cd/m².
pub fn lambertian_luminance(illuminance: f32) -> f32 {
    illuminance / std::f32::consts::PI
}

/// The histogram bin of a pre-exposed luminance (mirror of `luminance_bin` in the shader).
pub fn luminance_bin(pre_exposed: f32) -> usize {
    // NaN, zero and negatives land in the black bin.
    if pre_exposed.is_nan() || pre_exposed <= MIN_LOG2.exp2() {
        return 0;
    }
    let t = (pre_exposed.log2() - MIN_LOG2) / (MAX_LOG2 - MIN_LOG2);
    1 + ((t * (HISTOGRAM_BINS - 1) as f32) as usize).min(HISTOGRAM_BINS - 2)
}

/// Log2 of the pre-exposed luminance at the centre of a metered bin (1..).
fn bin_centre_log2(bin: usize) -> f32 {
    let width = (MAX_LOG2 - MIN_LOG2) / (HISTOGRAM_BINS - 1) as f32;
    MIN_LOG2 + (bin as f32 - 0.5) * width
}

/// One frame's luminance histogram and the exposure that frame was drawn with.
#[derive(Clone, Debug)]
pub struct LuminanceHistogram {
    /// Pixel counts per bin.
    pub bins: [u32; HISTOGRAM_BINS],
    /// The exposure the measured image was drawn with.
    pub exposure: f32,
}

impl LuminanceHistogram {
    /// A histogram built on the CPU from `(luminance in cd/m², pixel count)` pairs, as the
    /// GPU would bin them at `exposure` (tests, tools).
    pub fn from_samples(samples: impl IntoIterator<Item = (f32, u32)>, exposure: f32) -> Self {
        let mut bins = [0; HISTOGRAM_BINS];
        for (luminance, count) in samples {
            bins[luminance_bin(luminance * exposure)] += count;
        }
        Self { bins, exposure }
    }

    /// Pixels counted.
    pub fn samples(&self) -> u64 {
        self.bins.iter().map(|&c| u64::from(c)).sum()
    }

    /// Pixels brighter than the black bin.
    pub fn metered_samples(&self) -> u64 {
        self.samples() - u64::from(self.bins[0])
    }

    /// Log-average luminance in cd/m² of the metered samples between the `low` and `high`
    /// fractions of their sorted order (e.g. 0.5 and 0.98: the darker half and the
    /// brightest 2 % are left out). `None` when nothing was metered.
    pub fn average_luminance(&self, low: f32, high: f32) -> Option<f32> {
        let metered = self.metered_samples() as f64;
        if metered == 0.0 || self.exposure <= 0.0 {
            return None;
        }
        let (from, to) = (metered * f64::from(low), metered * f64::from(high));
        let mut below = 0.0;
        let mut sum = 0.0;
        let mut weight = 0.0;
        for (bin, &count) in self.bins.iter().enumerate().skip(1) {
            let count = f64::from(count);
            let taken = ((below + count).min(to) - below.max(from)).max(0.0);
            sum += taken * f64::from(bin_centre_log2(bin));
            weight += taken;
            below += count;
        }
        (weight > 0.0).then(|| ((sum / weight) as f32).exp2() / self.exposure)
    }
}

/// Exposure that follows the metered scene like an eye: EV100 moves towards the value the
/// histogram asks for, faster when the scene gets brighter than when it gets darker.
#[derive(Clone, Debug)]
pub struct AutoExposure {
    /// Current exposure value at ISO 100.
    pub ev100: f32,
    /// What the last histogram asked for (after compensation and clamping).
    pub target_ev100: f32,
    /// Adapt at all; when false `ev100` stays where it is set.
    pub automatic: bool,
    /// Exposure compensation in EV: positive makes the image brighter.
    pub compensation: f32,
    /// Lowest and highest EV100 the automatic mode may reach.
    pub range: (f32, f32),
    /// Fractions of the sorted metered samples that form the key (see
    /// [`LuminanceHistogram::average_luminance`]).
    pub band: (f32, f32),
    /// Adaptation rate per second towards a darker image (the scene got brighter).
    pub speed_darken: f32,
    /// Adaptation rate per second towards a brighter image (the scene got darker).
    pub speed_brighten: f32,
    started: bool,
}

impl AutoExposure {
    /// Automatic exposure starting at `ev100`; the first metered frame snaps to its target.
    pub fn new(ev100: f32) -> Self {
        Self {
            ev100,
            target_ev100: ev100,
            automatic: true,
            compensation: 0.0,
            range: (-6.0, 18.0),
            band: (0.5, 0.98),
            speed_darken: 1.5,
            speed_brighten: 0.8,
            started: false,
        }
    }

    /// A fixed exposure.
    pub fn fixed(ev100: f32) -> Self {
        Self {
            automatic: false,
            ..Self::new(ev100)
        }
    }

    /// Moves towards what `histogram` (if any) asks for, over `dt` seconds.
    pub fn update(&mut self, histogram: Option<&LuminanceHistogram>, dt: f32) {
        if !self.automatic {
            self.target_ev100 = self.ev100;
            return;
        }
        if let Some(luminance) =
            histogram.and_then(|h| h.average_luminance(self.band.0, self.band.1))
        {
            self.target_ev100 = (ev100_from_luminance(luminance) - self.compensation)
                .clamp(self.range.0, self.range.1);
            if !self.started {
                self.started = true;
                self.ev100 = self.target_ev100;
            }
        }
        let speed = if self.target_ev100 > self.ev100 {
            self.speed_darken
        } else {
            self.speed_brighten
        };
        self.ev100 += (self.target_ev100 - self.ev100) * (1.0 - (-dt * speed).exp());
    }

    /// The exposure multiplier the frame is drawn with.
    pub fn exposure(&self) -> f32 {
        exposure_from_ev100(self.ev100)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HistogramPush {
    bins: u64,
    image: u32,
    width: u32,
    height: u32,
    pad: u32,
}

/// Bytes of one histogram.
const HISTOGRAM_BYTES: u64 = (HISTOGRAM_BINS * 4) as u64;

/// The histogram passes, the device-local histogram and the per-slot readback buffers.
///
/// The GPU counts into device-local memory (atomics over PCIe into host memory cost
/// 0.4 ms here) and copies the 1 KB result into a host-cached buffer per frame slot (reading
/// host-visible video memory through Resizable BAR cost the CPU 0.02 ms per frame).
pub struct LuminanceMeter {
    pipeline: Pipeline,
    bins: GraphBuffer,
    readback: Vec<GraphBuffer>,
    /// Exposure each slot's last histogram was measured with; 0 = nothing measured yet.
    measured_with: [f32; FRAMES_IN_FLIGHT],
}

impl LuminanceMeter {
    /// Compiles the pass and creates the histogram and one readback buffer per frame in
    /// flight.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("exposure.slang", "histogram_main", ShaderStage::Compute)?,
            "luminance histogram",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "histogram_main"),
            push_constant_bytes: std::mem::size_of::<HistogramPush>() as u32,
            name: "luminance histogram",
        })?;
        device.destroy_shader_module(module);
        let bins = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: HISTOGRAM_BYTES,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_SRC
                | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "luminance histogram",
        })?);
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: HISTOGRAM_BYTES,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("luminance histogram readback {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline,
            bins,
            readback,
            measured_with: [0.0; FRAMES_IN_FLIGHT],
        })
    }

    /// The histogram the frame that last used `slot` measured (its commands have
    /// completed); `None` before any.
    pub fn take(&mut self, slot: FrameSlot) -> Option<LuminanceHistogram> {
        let exposure = std::mem::take(&mut self.measured_with[slot.index]);
        if exposure <= 0.0 {
            return None;
        }
        let mut bins = [0_u32; HISTOGRAM_BINS];
        self.readback[slot.index].read(0, &mut bins);
        Some(LuminanceHistogram { bins, exposure })
    }

    /// Declares the passes "exposure/luminance histogram" over `color`, the pre-exposed HDR
    /// image of this frame, drawn with `exposure`: clear the bins, count, copy the bins to
    /// this slot's readback buffer and hand it to the host.
    pub fn measure<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        color: ImageHandle,
        extent: vk::Extent2D,
        exposure: f32,
    ) {
        const LABEL: &str = "exposure/luminance histogram";
        self.measured_with[slot.index] = exposure;
        let this: &'f Self = self;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let (bins_buffer, readback_buffer) = (&this.bins, &this.readback[slot.index]);
        let bins = graph.import_buffer(bins_buffer);
        let readback = graph.import_buffer(readback_buffer);
        let pipeline = &this.pipeline;
        graph
            .pass(LABEL)
            .buffer(bins, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.fill_buffer(bins_buffer, 0, HISTOGRAM_BYTES, 0);
                Ok(())
            });
        graph
            .pass(LABEL)
            .image(color, ImageAccess::Sampled(compute))
            .buffer(bins, BufferAccess::ShaderReadWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &HistogramPush {
                        bins: bins_buffer.address(),
                        image: resources.sampled(color).0,
                        width: extent.width,
                        height: extent.height,
                        pad: 0,
                    },
                );
                commands.dispatch(extent.width.div_ceil(16), extent.height.div_ceil(16), 1);
                Ok(())
            });
        graph
            .pass(LABEL)
            .buffer(bins, BufferAccess::TransferSrc)
            .buffer(readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(bins_buffer, readback_buffer, HISTOGRAM_BYTES);
                Ok(())
            });
        graph
            .pass(LABEL)
            .buffer(readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sunlit_grey_card_meters_at_sunny_16() {
        // Sunny 16 at ISO 100: f/16 at 1/100 s, EV 14.6; a meter reads the grey card at noon.
        let card = 0.18 * lambertian_luminance(100_000.0);
        let ev = ev100_from_luminance(card);
        assert!((14.5..16.0).contains(&ev), "{ev}");
        // The metered average lands at 1 / 9.6 of the clip point once exposed.
        let exposed = card * exposure_from_ev100(ev);
        assert!((exposed - 1.0 / 9.6).abs() < 1e-4, "{exposed}");
    }

    #[test]
    fn bins_cover_the_range_and_send_black_and_nan_to_bin_zero() {
        assert_eq!(luminance_bin(0.0), 0);
        assert_eq!(luminance_bin(-1.0), 0);
        assert_eq!(luminance_bin(f32::NAN), 0);
        assert_eq!(luminance_bin(1e-6), 0);
        assert_eq!(luminance_bin(1e9), HISTOGRAM_BINS - 1);
        let mut last = 0;
        for i in 0..400 {
            let bin = luminance_bin((MIN_LOG2 + 0.01 + i as f32 * 0.06).exp2());
            assert!(bin >= last && bin >= 1);
            last = bin;
        }
        // A bin's centre maps back into that bin.
        for bin in 1..HISTOGRAM_BINS {
            assert_eq!(luminance_bin(bin_centre_log2(bin).exp2()), bin);
        }
    }

    #[test]
    fn a_uniform_scene_is_metered_at_its_luminance() {
        for (luminance, exposure) in [(3000.0, 2.5e-5), (0.05, 0.3), (1.0e6, 1.0e-7)] {
            let h = LuminanceHistogram::from_samples([(luminance, 1000)], exposure);
            let metered = h.average_luminance(0.5, 0.98).expect("metered");
            // Within half a bin (0.047 EV).
            assert!(
                (metered / luminance).log2().abs() < 0.05,
                "{luminance} → {metered}"
            );
        }
    }

    #[test]
    fn black_pixels_are_not_metered() {
        // Two thirds of the view is black space: the rock alone sets the key.
        let rock = 4000.0;
        let h = LuminanceHistogram::from_samples([(0.0, 2000), (rock, 1000)], 2.5e-5);
        assert_eq!(h.metered_samples(), 1000);
        let metered = h.average_luminance(0.0, 1.0).expect("metered");
        assert!((metered / rock).log2().abs() < 0.05);
        let empty = LuminanceHistogram::from_samples([(0.0, 10)], 1.0);
        assert!(empty.average_luminance(0.0, 1.0).is_none());
    }

    #[test]
    fn the_band_leaves_out_the_darker_part_and_the_brightest_highlights() {
        // Dim sky (half the view), a lit rock and 2 % of sun: the band 0.5..0.98 meters the
        // rock alone, where the whole range would sit between the sky and the sun.
        let h =
            LuminanceHistogram::from_samples([(100.0, 500), (5000.0, 480), (1.0e9, 20)], 2.5e-5);
        let metered = h.average_luminance(0.5, 0.98).expect("metered");
        assert!((metered / 5000.0).log2().abs() < 0.05, "{metered}");
        let everything = h.average_luminance(0.0, 1.0).expect("metered");
        assert!((everything / 5000.0).log2().abs() > 1.0, "{everything}");
    }

    #[test]
    fn adaptation_snaps_first_then_converges_without_overshoot() {
        let mut auto = AutoExposure::new(10.0);
        let bright = LuminanceHistogram::from_samples([(4000.0, 100)], 1.0e-4);
        auto.update(Some(&bright), 1.0 / 60.0);
        let target = ev100_from_luminance(4000.0);
        assert!(
            (auto.ev100 - target).abs() < 0.05,
            "snapped to {}",
            auto.ev100
        );
        // The scene gets 4 EV darker: EV100 falls towards the new target, monotonically.
        let dark = LuminanceHistogram::from_samples([(250.0, 100)], 1.0e-4);
        let new_target = ev100_from_luminance(250.0);
        let mut previous = auto.ev100;
        for _ in 0..600 {
            auto.update(Some(&dark), 1.0 / 60.0);
            assert!(auto.ev100 <= previous && auto.ev100 >= new_target - 1e-3);
            previous = auto.ev100;
        }
        assert!((auto.ev100 - new_target).abs() < 0.01, "{}", auto.ev100);
        // Compensation and the clamp.
        auto.compensation = 1.0;
        auto.range = (0.0, 12.0);
        auto.update(Some(&bright), 1.0 / 60.0);
        assert!((auto.target_ev100 - (target - 1.0).min(12.0)).abs() < 0.05);
    }

    #[test]
    fn a_fixed_exposure_does_not_move() {
        let mut fixed = AutoExposure::fixed(15.0);
        let h = LuminanceHistogram::from_samples([(10.0, 100)], 1.0);
        fixed.update(Some(&h), 1.0);
        assert_eq!(fixed.ev100, 15.0);
        assert!((fixed.exposure() - exposure_from_ev100(15.0)).abs() < 1e-12);
    }
}

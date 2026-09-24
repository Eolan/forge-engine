//! The frame profile shown by the overlay: named CPU and GPU zones per frame, smoothed and
//! grouped by subject (`group/name` labels), the device's memory (issue #9) and counters the
//! demo reports.

use std::collections::VecDeque;

use forge_gpu::{BUDGET_WARNING, GpuZone, MemoryCategory, MemoryReport, vk};

use crate::overlay::{Canvas, Color};

const HISTORY: usize = 240;
/// Weight of the newest sample in the running average.
const SMOOTHING: f64 = 0.08;
const NAME_COLUMNS: usize = 32;
const BAR_CELLS: usize = 16;
/// The fold group of the memory counters, after the timing groups.
const MEMORY_GROUP: &str = "memory";
const MIB: f64 = (1 << 20) as f64;
const GIB: f64 = (1 << 30) as f64;

/// How much of the profile the overlay shows (F1 cycles through these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayMode {
    /// Nothing drawn.
    Off,
    /// The header and one line per group; a digit key unfolds that group.
    Compact,
    /// Every zone and the counters; a digit key folds that group.
    Full,
}

/// The memory counters: the device's report and the traffic measured since the previous one.
#[derive(Clone, Debug)]
pub struct MemorySample {
    /// The device's report.
    pub report: MemoryReport,
    /// Host bytes written into GPU-visible memory per frame, averaged since the previous
    /// sample.
    pub uploaded_per_frame: f64,
    /// The same per second.
    pub uploaded_per_second: f64,
    /// Host bytes read back from GPU-visible memory per frame.
    pub read_back_per_frame: f64,
}

impl MemorySample {
    /// One line for logs: VRAM against the budget, the categories, the traffic.
    pub fn summary(&self) -> String {
        let report = &self.report;
        let mut line = match report.device_local() {
            Some((usage, budget)) => format!(
                "VRAM {:.1} of {:.1} MiB ({:.1}%)",
                usage as f64 / MIB,
                budget as f64 / MIB,
                100.0 * usage as f64 / budget.max(1) as f64
            ),
            None => "VRAM usage unknown (no VK_EXT_memory_budget)".to_owned(),
        };
        line += &format!(
            "; allocated {:.2} MiB in {:.1} MiB of blocks:",
            report.total_allocated() as f64 / MIB,
            report.reserved as f64 / MIB
        );
        for category in MemoryCategory::ALL {
            line += &format!(
                " {} {:.2},",
                category.name(),
                report.allocated(category) as f64 / MIB
            );
        }
        if let Some(outside) = report.outside_allocator() {
            line += &format!(" outside the allocator {:.1} MiB", outside as f64 / MIB);
        }
        line + &format!(
            "; uploads {:.2} KiB/frame ({:.2} MiB/s), read back {:.2} KiB/frame",
            self.uploaded_per_frame / 1024.0,
            self.uploaded_per_second / MIB,
            self.read_back_per_frame / 1024.0
        )
    }
}

struct Zone {
    label: String,
    ms: f64,
    seen: bool,
}

/// Per-frame timings and counters, kept by the shell and read by the overlay.
pub struct Profile {
    /// What the overlay shows.
    pub mode: OverlayMode,
    frame_ms: VecDeque<f64>,
    gpu: Vec<Zone>,
    cpu: Vec<Zone>,
    counters: Vec<String>,
    memory: Option<MemorySample>,
    /// Groups whose fold state the digit keys flipped from the mode's default.
    toggled: Vec<String>,
    /// GPU milliseconds per zone summed over the run (unsmoothed), for the exit log.
    run_gpu: Vec<(String, f64)>,
    /// Frames whose GPU zones were summed.
    run_frames: u64,
}

impl Profile {
    pub(crate) fn new(mode: OverlayMode) -> Self {
        Self {
            mode,
            frame_ms: VecDeque::with_capacity(HISTORY),
            gpu: Vec::new(),
            cpu: Vec::new(),
            counters: Vec::new(),
            memory: None,
            toggled: Vec::new(),
            run_gpu: Vec::new(),
            run_frames: 0,
        }
    }

    /// The GPU zones averaged over every frame of the run so far, largest first: the line
    /// the shell logs at exit (the overlay shows smoothed values of the last frames only).
    pub fn gpu_run_summary(&self) -> Option<String> {
        if self.run_frames == 0 {
            return None;
        }
        let frames = self.run_frames as f64;
        let mut zones: Vec<(&str, f64)> = self
            .run_gpu
            .iter()
            .map(|(label, sum)| (label.as_str(), sum / frames))
            .collect();
        zones.sort_by(|a, b| b.1.total_cmp(&a.1));
        let total: f64 = zones.iter().map(|z| z.1).sum();
        let mut line = format!("{total:.3} ms per frame over {} frames:", self.run_frames);
        for (label, ms) in zones {
            line += &format!(" {label} {ms:.3},");
        }
        line.pop();
        Some(line)
    }

    /// The latest memory counters (refreshed four times per second).
    pub fn memory(&self) -> Option<&MemorySample> {
        self.memory.as_ref()
    }

    pub(crate) fn set_memory(&mut self, sample: MemorySample) {
        self.memory = Some(sample);
    }

    /// Whether anything is drawn.
    pub fn is_visible(&self) -> bool {
        self.mode != OverlayMode::Off
    }

    /// Off → compact → full → off.
    pub fn cycle_mode(&mut self) {
        self.mode = match self.mode {
            OverlayMode::Off => OverlayMode::Compact,
            OverlayMode::Compact => OverlayMode::Full,
            OverlayMode::Full => OverlayMode::Off,
        };
        self.toggled.clear();
    }

    fn folded(&self, group: &str) -> bool {
        let toggled = self.toggled.iter().any(|g| g == group);
        match self.mode {
            OverlayMode::Compact => !toggled,
            _ => toggled,
        }
    }

    /// Adds a line to the counters block for this frame (demos call it every frame).
    pub fn counter(&mut self, line: impl Into<String>) {
        self.counters.push(line.into());
    }

    pub(crate) fn begin_frame(&mut self) {
        self.counters.clear();
        for zone in &mut self.cpu {
            zone.seen = false;
        }
    }

    pub(crate) fn frame_time(&mut self, ms: f64) {
        if self.frame_ms.len() == HISTORY {
            self.frame_ms.pop_front();
        }
        self.frame_ms.push_back(ms);
    }

    pub(crate) fn cpu_zone(&mut self, label: &'static str, ms: f64) {
        upsert(&mut self.cpu, label, ms);
    }

    pub(crate) fn gpu_zones(&mut self, zones: &[GpuZone]) {
        if zones.is_empty() {
            return;
        }
        for zone in &mut self.gpu {
            zone.seen = false;
        }
        for zone in zones {
            upsert(&mut self.gpu, zone.label, zone.ms);
            match self
                .run_gpu
                .iter_mut()
                .find(|(label, _)| label == zone.label)
            {
                Some((_, sum)) => *sum += zone.ms,
                None => self.run_gpu.push((zone.label.to_owned(), zone.ms)),
            }
        }
        self.run_frames += 1;
        self.gpu.retain(|z| z.seen);
    }

    /// Folds or unfolds the `index`-th group of the display (the digit keys).
    pub(crate) fn toggle_group(&mut self, index: usize) {
        let groups = self.groups();
        if let Some(name) = groups.get(index) {
            if let Some(pos) = self.toggled.iter().position(|g| g == name) {
                self.toggled.remove(pos);
            } else {
                self.toggled.push(name.clone());
            }
        }
    }

    /// Group names in display order: GPU groups by first appearance, the CPU loop, memory.
    fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = Vec::new();
        for zone in self.gpu.iter().chain(self.cpu.iter()) {
            let group = group_of(&zone.label).to_owned();
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        if self.memory.is_some() {
            groups.push(MEMORY_GROUP.to_owned());
        }
        groups
    }

    fn percentiles(&self) -> (f64, f64, f64) {
        if self.frame_ms.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut sorted: Vec<f64> = self.frame_ms.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
        let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
        (mean, at(0.5), at(0.99))
    }

    /// Draws the profile into `canvas`.
    pub(crate) fn layout(&self, canvas: &mut Canvas, title: &str, extent: vk::Extent2D) {
        let (mean, p50, p99) = self.percentiles();
        let gpu_total: f64 = self.gpu.iter().map(|z| z.ms).sum();
        let cpu_total: f64 = self.cpu.iter().filter(|z| !is_wait(z)).map(|z| z.ms).sum();
        let width = canvas.cols().min(NAME_COLUMNS + BAR_CELLS + 30);
        let groups = self.groups();

        let show_counters = self.mode == OverlayMode::Full && !self.counters.is_empty();
        // Count the rows first so the panel fits.
        let mut rows = 3;
        for group in groups.iter().filter(|g| *g != MEMORY_GROUP) {
            rows += 1;
            if !self.folded(group) {
                rows += self.zones_in(group).count();
            }
        }
        if show_counters {
            rows += 1 + self.counters.len();
        }
        rows += 1;
        // The memory group goes under the timings, or beside them when it would push the
        // counters off the bottom and the screen is wide enough.
        let memory_rows = self.memory_group_rows();
        let beside =
            memory_rows > 0 && rows + memory_rows > canvas.rows() && canvas.cols() > 2 * width;
        if !beside {
            rows += memory_rows;
        }
        canvas.panel(0, 0, width, rows.min(canvas.rows()), Color::Panel);
        canvas.panel(0, 0, width, 2, Color::Header);

        let fps = if mean > 0.0 { 1000.0 / mean } else { 0.0 };
        let header = format!(
            "{title}   {}x{}   frame {mean:.2} ms ({fps:.0} fps)   p50 {p50:.2}   p99 {p99:.2}",
            extent.width, extent.height
        );
        canvas.text(1, 0, &clip(&header, width - 2), Color::White);
        let totals = format!(
            "GPU {gpu_total:.2} ms   CPU {cpu_total:.2} ms (main thread, without the wait)"
        );
        canvas.text(1, 1, &clip(&totals, width - 2), Color::Grey);
        let keys = match self.mode {
            OverlayMode::Compact => "F1 more  1-9 open",
            _ => "F1 hide  1-9 fold",
        };
        canvas.text(width.saturating_sub(keys.len() + 1), 1, keys, Color::Blue);

        let mut row = 3;
        for (index, group) in groups.iter().enumerate() {
            if group == MEMORY_GROUP {
                if beside {
                    let col = width + 1;
                    canvas.panel(col, 2, width, memory_rows + 2, Color::Panel);
                    self.layout_memory(canvas, col + 1, 3, index);
                } else {
                    row = self.layout_memory(canvas, 1, row, index);
                }
                continue;
            }
            let is_cpu = group == "cpu";
            let total: f64 = self
                .zones_in(group)
                .filter(|z| !is_wait(z))
                .map(|z| z.ms)
                .sum();
            let reference = if is_cpu {
                mean.max(1e-6)
            } else {
                gpu_total.max(1e-6)
            };
            let folded = self.folded(group);
            let heading = format!(
                "{}{} {:<width$} {total:7.2} ms {:3.0}%",
                if folded { "+" } else { "-" },
                index + 1,
                clip(&group.to_uppercase(), NAME_COLUMNS - 4),
                100.0 * total / reference,
                width = NAME_COLUMNS - 4
            );
            canvas.text(1, row, &heading, Color::Yellow);
            canvas.bar(
                NAME_COLUMNS + 18,
                row,
                BAR_CELLS,
                (total / reference) as f32,
                heat(total / reference),
            );
            row += 1;
            if folded {
                continue;
            }
            for zone in self.zones_in(group) {
                let share = zone.ms / reference;
                let line = format!(
                    "     {:<width$} {:7.2} ms {:3.0}%",
                    clip(name_of(&zone.label), NAME_COLUMNS - 5),
                    zone.ms,
                    100.0 * share,
                    width = NAME_COLUMNS - 5
                );
                canvas.text(
                    1,
                    row,
                    &line,
                    if is_wait(zone) {
                        Color::Dim
                    } else {
                        Color::Grey
                    },
                );
                canvas.bar(
                    NAME_COLUMNS + 18,
                    row,
                    BAR_CELLS,
                    share as f32,
                    if is_wait(zone) {
                        Color::Track
                    } else {
                        heat(share)
                    },
                );
                row += 1;
            }
        }
        if show_counters {
            canvas.text(1, row, "COUNTERS", Color::Yellow);
            row += 1;
            for line in &self.counters {
                canvas.text(6, row, line, Color::Grey);
                row += 1;
            }
        }
    }

    /// Lines of the unfolded memory group: the heaps, the categories, the memory outside the
    /// allocator, its blocks and the traffic.
    fn memory_rows(&self) -> usize {
        self.memory
            .as_ref()
            .map_or(0, |m| m.report.heaps.len() + MemoryCategory::COUNT + 4)
    }

    /// Rows of the memory group with its heading (0 without a sample).
    fn memory_group_rows(&self) -> usize {
        match self.memory {
            Some(_) if self.folded(MEMORY_GROUP) => 1,
            Some(_) => 1 + self.memory_rows(),
            None => 0,
        }
    }

    /// Draws the memory group, the `index`-th, from `col` and `row`; returns the row after it.
    fn layout_memory(
        &self,
        canvas: &mut Canvas,
        col: usize,
        mut row: usize,
        index: usize,
    ) -> usize {
        let Some(sample) = &self.memory else {
            return row;
        };
        let bars = col - 1 + NAME_COLUMNS + 18;
        let report = &sample.report;
        let (value, share) = match report.device_local() {
            Some((usage, budget)) => (usage, Some(usage as f64 / budget.max(1) as f64)),
            None => (report.total_allocated(), None),
        };
        let folded = self.folded(MEMORY_GROUP);
        let (value, unit) = size(value);
        let heading = format!(
            "{}{} {:<width$} {value:>6} {unit}{}",
            if folded { "+" } else { "-" },
            index + 1,
            "MEMORY (VRAM, % OF BUDGET)",
            percent(share),
            width = NAME_COLUMNS - 4
        );
        let warning = share.is_some_and(|s| s >= BUDGET_WARNING);
        let heading_color = if warning { Color::Red } else { Color::Yellow };
        canvas.text(col, row, &heading, heading_color);
        if let Some(share) = share {
            let color = budget_heat(share);
            canvas.bar(bars, row, BAR_CELLS, share as f32, color);
        }
        row += 1;
        if folded {
            return row;
        }
        for heap in &report.heaps {
            let (budget, unit) = size(heap.budget);
            let name = format!("{} of {budget} {unit}", heap.name());
            let share = heap.usage.map(|u| u as f64 / heap.budget.max(1) as f64);
            let warning = share.is_some_and(|s| s >= BUDGET_WARNING);
            let line = memory_line(&name, heap.usage.map(size), &percent(share));
            let color = if warning { Color::Red } else { Color::Grey };
            canvas.text(col, row, &line, color);
            if let Some(share) = share {
                let color = budget_heat(share);
                canvas.bar(bars, row, BAR_CELLS, share as f32, color);
            }
            row += 1;
        }
        let total = report.total_allocated().max(1) as f64;
        for category in MemoryCategory::ALL {
            let bytes = report.allocated(category);
            let share = bytes as f64 / total;
            let line = memory_line(category.name(), Some(size(bytes)), &percent(Some(share)));
            canvas.text(col, row, &line, Color::Grey);
            canvas.bar(bars, row, BAR_CELLS, share as f32, heat(share));
            row += 1;
        }
        let outside = report.outside_allocator().map(size);
        let filled = percent(Some(total / report.reserved.max(1) as f64));
        let kib = |bytes: f64| (format!("{:.2}", bytes / 1024.0), "KiB");
        let upload_rate = format!(" {:.2} MiB/s", sample.uploaded_per_second / MIB);
        let lines = [
            (
                memory_line("driver, swapchain, others", outside, ""),
                Color::Dim,
            ),
            (
                memory_line(
                    "allocator blocks, % used",
                    Some(size(report.reserved)),
                    &filled,
                ),
                Color::Dim,
            ),
            (
                memory_line(
                    "uploads per frame",
                    Some(kib(sample.uploaded_per_frame)),
                    &upload_rate,
                ),
                Color::Grey,
            ),
            (
                memory_line(
                    "read back per frame",
                    Some(kib(sample.read_back_per_frame)),
                    "",
                ),
                Color::Grey,
            ),
        ];
        for (line, color) in lines {
            canvas.text(col, row, &line, color);
            row += 1;
        }
        row
    }

    fn zones_in<'a>(&'a self, group: &'a str) -> impl Iterator<Item = &'a Zone> + 'a {
        self.gpu
            .iter()
            .chain(self.cpu.iter())
            .filter(move |z| group_of(&z.label) == group)
    }
}

fn upsert(list: &mut Vec<Zone>, label: &str, ms: f64) {
    match list.iter_mut().find(|z| z.label == label) {
        Some(zone) => {
            zone.ms += (ms - zone.ms) * SMOOTHING;
            zone.seen = true;
        }
        None => list.push(Zone {
            label: label.to_owned(),
            ms,
            seen: true,
        }),
    }
}

fn group_of(label: &str) -> &str {
    label.split_once('/').map_or("other", |(group, _)| group)
}

/// The main thread blocked on the GPU: shown, but not counted as CPU work.
fn is_wait(zone: &Zone) -> bool {
    zone.label.contains("wait for GPU")
}

/// At most `width` characters.
fn clip(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

fn name_of(label: &str) -> &str {
    label.split_once('/').map_or(label, |(_, name)| name)
}

/// A size in six characters or fewer with its unit: MiB, or GiB from 10 000 MiB.
fn size(bytes: u64) -> (String, &'static str) {
    let mib = bytes as f64 / MIB;
    if mib >= 10_000.0 {
        (format!("{:.2}", bytes as f64 / GIB), "GiB")
    } else {
        (format!("{mib:.1}"), "MiB")
    }
}

/// A memory line whose percentage falls in the zone lines' column: the name, the value and
/// its unit (`?` when unknown), a note.
fn memory_line(name: &str, value: Option<(String, &str)>, note: &str) -> String {
    let (value, unit) = value.unwrap_or_else(|| ("?".to_owned(), "MiB"));
    format!(
        "     {:<width$} {value:>6} {unit}{note}",
        clip(name, NAME_COLUMNS - 5),
        width = NAME_COLUMNS - 5
    )
}

/// `"  42%"` (with the space before it), or blanks when there is nothing to divide by.
fn percent(share: Option<f64>) -> String {
    share.map_or_else(|| "     ".to_owned(), |s| format!(" {:3.0}%", 100.0 * s))
}

/// Bar colour of a share of a memory budget: warm from three quarters, the warning colour
/// from [`BUDGET_WARNING`] (the 10 % reserve of D-018).
fn budget_heat(share: f64) -> Color {
    if share >= BUDGET_WARNING {
        Color::Red
    } else if share >= 0.75 {
        Color::Orange
    } else {
        Color::Cyan
    }
}

/// Bar colour by share of the reference: cool below a quarter, warm below a half, hot above.
fn heat(share: f64) -> Color {
    if share >= 0.5 {
        Color::Red
    } else if share >= 0.25 {
        Color::Orange
    } else {
        Color::Cyan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_summary_averages_every_zone_over_all_frames_largest_first() {
        let mut profile = Profile::new(OverlayMode::Off);
        assert_eq!(profile.gpu_run_summary(), None);
        let zone = |label, ms| GpuZone {
            label,
            ms,
            start_ticks: 0,
            end_ticks: 0,
        };
        profile.gpu_zones(&[zone("shading/resolve", 0.1), zone("geometry/cull", 0.2)]);
        // A zone missing from a frame counts as zero there; a repeated label adds up.
        profile.gpu_zones(&[zone("geometry/cull", 0.3), zone("geometry/cull", 0.1)]);
        assert_eq!(
            profile.gpu_run_summary().as_deref(),
            Some("0.350 ms per frame over 2 frames: geometry/cull 0.300, shading/resolve 0.050")
        );
    }

    #[test]
    fn zones_group_by_prefix_and_smooth() {
        let mut profile = Profile::new(OverlayMode::Full);
        let zone = |label, ms| GpuZone {
            label,
            ms,
            start_ticks: 0,
            end_ticks: 0,
        };
        profile.gpu_zones(&[zone("geometry/pass 1", 4.0), zone("temporal/resolve", 1.0)]);
        profile.cpu_zone("cpu/update", 0.1);
        assert_eq!(profile.groups(), vec!["geometry", "temporal", "cpu"]);
        profile.gpu_zones(&[zone("geometry/pass 1", 8.0)]);
        // The first sample is taken as is, the second moves it by the smoothing weight.
        let pass1 = profile
            .gpu
            .iter()
            .find(|z| z.label == "geometry/pass 1")
            .unwrap();
        assert!((pass1.ms - (4.0 + 4.0 * SMOOTHING)).abs() < 1e-9);
        // A zone that stopped appearing is dropped.
        assert!(profile.gpu.iter().all(|z| z.label != "temporal/resolve"));
        profile.toggle_group(0);
        assert!(profile.folded("geometry"));
        assert!(!profile.folded("temporal"));
        profile.toggle_group(0);
        assert!(!profile.folded("geometry"));
        profile.cycle_mode();
        assert_eq!(profile.mode, OverlayMode::Off);
        profile.cycle_mode();
        assert_eq!(profile.mode, OverlayMode::Compact);
        assert!(profile.folded("geometry"));
        profile.toggle_group(0);
        assert!(!profile.folded("geometry"));
    }

    fn sample(vram_usage: u64) -> MemorySample {
        let heap = |index, device_local, usage, budget| forge_gpu::HeapReport {
            index,
            size: budget,
            device_local,
            host_visible: true,
            usage: Some(usage),
            budget,
        };
        let mut allocated = [0; MemoryCategory::COUNT];
        allocated[0] = 300 << 20;
        allocated[1] = 100 << 20;
        MemorySample {
            report: MemoryReport {
                heaps: vec![
                    heap(0, true, vram_usage, 16 << 30),
                    heap(1, false, 64 << 20, 32 << 30),
                ],
                allocated,
                reserved: 512 << 20,
                uploaded: 0,
                read_back: 0,
            },
            uploaded_per_frame: 2048.0,
            uploaded_per_second: 2048.0 * 400.0,
            read_back_per_frame: 64.0,
        }
    }

    #[test]
    fn memory_is_the_last_group_and_keeps_the_zone_columns() {
        let mut profile = Profile::new(OverlayMode::Compact);
        profile.cpu_zone("cpu/update", 0.1);
        assert_eq!(profile.groups(), vec!["cpu"]);
        profile.set_memory(sample(1 << 30));
        assert_eq!(profile.groups(), vec!["cpu", "memory"]);
        assert!(profile.folded("memory"));
        profile.toggle_group(1);
        assert!(!profile.folded("memory"));
        assert_eq!(profile.memory_rows(), 2 + MemoryCategory::COUNT + 4);
        // The percentages line up with the timings'.
        let zone = format!("     {:<27} {:7.2} ms {:3.0}%", "pass", 1.0, 5.0);
        let memory = memory_line("geometry", Some(size(300 << 20)), &percent(Some(0.73)));
        assert_eq!(
            memory.find('%'),
            zone.find('%'),
            "{memory}
{zone}"
        );
        assert_eq!(memory.len(), zone.len());
        assert_eq!(size(20_000 << 20), ("19.53".to_owned(), "GiB"));
        let summary = profile.memory().unwrap().summary();
        assert!(
            summary.starts_with("VRAM 1024.0 of 16384.0 MiB (6.2%)"),
            "{summary}"
        );
        assert!(summary.contains("geometry 300.00"), "{summary}");
    }

    #[test]
    fn memory_turns_to_the_warning_colour_within_ten_percent_of_the_budget() {
        assert_eq!(budget_heat(0.5), Color::Cyan);
        assert_eq!(budget_heat(0.8), Color::Orange);
        assert_eq!(budget_heat(0.9), Color::Red);
        for (usage, color) in [(8 << 30, Color::Yellow), (15 << 30, Color::Red)] {
            let mut profile = Profile::new(OverlayMode::Full);
            profile.set_memory(sample(usage));
            let mut canvas = Canvas::blank(120, 40);
            profile.layout(&mut canvas, "test", vk::Extent2D::default());
            let (heading, heading_color) = canvas.row_text(3);
            assert!(heading.contains("MEMORY"), "{heading}");
            assert_eq!(heading_color, Some(color as u32), "{heading}");
            let (heap, _) = canvas.row_text(4);
            assert!(heap.contains("VRAM (ReBAR) of 16.00 GiB"), "{heap}");
            let (uploads, _) = canvas.row_text(4 + 2 + MemoryCategory::COUNT + 2);
            assert!(uploads.contains("uploads per frame"), "{uploads}");
            assert!(uploads.contains("2.00 KiB 0.78 MiB/s"), "{uploads}");
        }
    }

    #[test]
    fn memory_moves_beside_the_timings_when_it_would_not_fit_under_them() {
        let mut profile = Profile::new(OverlayMode::Full);
        profile.cpu_zone("cpu/update", 0.1);
        profile.set_memory(sample(1 << 30));
        let mut tall = Canvas::blank(200, 40);
        profile.layout(&mut tall, "test", vk::Extent2D::default());
        let (under, _) = tall.row_text(5);
        assert_eq!(under.find("MEMORY"), Some(4), "{under}");
        let mut short = Canvas::blank(200, 12);
        profile.layout(&mut short, "test", vk::Extent2D::default());
        let (cpu, _) = short.row_text(3);
        let width = NAME_COLUMNS + BAR_CELLS + 30;
        assert_eq!(cpu.find("CPU"), Some(4), "{cpu}");
        assert_eq!(cpu.find("MEMORY"), Some(width + 2 + 3), "{cpu}");
        // Too narrow for two panels: under the timings, clipped at the bottom.
        let mut narrow = Canvas::blank(120, 12);
        profile.layout(&mut narrow, "test", vk::Extent2D::default());
        let (under, _) = narrow.row_text(5);
        assert_eq!(under.find("MEMORY"), Some(4), "{under}");
    }
}

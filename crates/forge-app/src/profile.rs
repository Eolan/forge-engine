//! The frame profile shown by the overlay: named CPU and GPU zones per frame, smoothed and
//! grouped by subject (`group/name` labels), plus counters the demo reports.

use std::collections::VecDeque;

use forge_gpu::{GpuZone, vk};

use crate::overlay::{Canvas, Color};

const HISTORY: usize = 240;
/// Weight of the newest sample in the running average.
const SMOOTHING: f64 = 0.08;
const NAME_COLUMNS: usize = 32;
const BAR_CELLS: usize = 16;

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
    /// Groups whose fold state the digit keys flipped from the mode's default.
    toggled: Vec<String>,
}

impl Profile {
    pub(crate) fn new(mode: OverlayMode) -> Self {
        Self {
            mode,
            frame_ms: VecDeque::with_capacity(HISTORY),
            gpu: Vec::new(),
            cpu: Vec::new(),
            counters: Vec::new(),
            toggled: Vec::new(),
        }
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
        }
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

    /// Group names in display order: GPU groups by first appearance, then the CPU loop.
    fn groups(&self) -> Vec<String> {
        let mut groups: Vec<String> = Vec::new();
        for zone in self.gpu.iter().chain(self.cpu.iter()) {
            let group = group_of(&zone.label).to_owned();
            if !groups.contains(&group) {
                groups.push(group);
            }
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
        for group in &groups {
            rows += 1;
            if !self.folded(group) {
                rows += self.zones_in(group).count();
            }
        }
        if show_counters {
            rows += 1 + self.counters.len();
        }
        rows += 1;
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
}

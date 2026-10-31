//! The settings screen's rows (#377): which settings each tab shows, their choices in words, and
//! whether this machine and this demo have them.
//!
//! A row is built from the settings registered in the run: a demo that registers none of a
//! row's settings has no such row, and a choice the machine cannot take (DLSS without an RTX GPU)
//! stays listed but is skipped when stepping, its reason in the description line. Each choice
//! sets one or more settings ([`Choice::sets`]): the anti-aliasing and upscaling row sets `r.aa`
//! and the upscaler's mode together.

use std::collections::{BTreeMap, BTreeSet};

use forge_core::settings::{Flags, Settings, Source};

use crate::presets;

/// What the shell knows that the settings don't: the GPU's upscalers, the monitors, the display.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caps {
    /// The device runs DLSS (an RTX GPU, NVIDIA's Streamline loaded at start).
    pub dlss: bool,
    /// The monitors, in the system's order.
    pub monitors: Vec<Monitor>,
    /// The window's monitor, an index into `monitors`.
    pub monitor: Option<usize>,
    /// The window's display is in HDR (Windows' "Use HDR").
    pub hdr_display: bool,
    /// The output is HDR now (the calibration pages need it).
    pub hdr_output: bool,
    /// The window's size now, pixels.
    pub window: [u32; 2],
}

impl Caps {
    /// The size of the window's monitor, or of the first one.
    fn monitor_size(&self) -> Option<[u32; 2]> {
        self.monitor
            .and_then(|i| self.monitors.get(i))
            .or(self.monitors.first())
            .map(|m| m.size)
    }
}

/// A monitor, as the menu lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    /// Its size, pixels.
    pub size: [u32; 2],
    /// Whether it is the system's main monitor.
    pub primary: bool,
}

/// The settings screen's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// The look and its cost.
    #[default]
    Graphics,
    /// The window, the monitor and the frame's pace.
    Display,
    /// The sound and the mouse.
    Game,
}

impl Tab {
    /// Every tab, in order.
    pub const ALL: [Self; 3] = [Self::Graphics, Self::Display, Self::Game];

    /// Its name on the screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Graphics => "Graphics",
            Self::Display => "Display",
            Self::Game => "Sound and controls",
        }
    }
}

/// One of a row's choices.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// Its words.
    pub label: String,
    /// The settings it sets and their values.
    pub sets: Vec<(&'static str, String)>,
    /// How many of `sets`, from the first, say that the choice is the one in force: the quality
    /// preset is chosen by `r.preset` alone, though choosing it sets its patch too.
    pub identify: usize,
    /// Whether this machine and this run can take it.
    pub available: bool,
    /// A line about it for the description, or why it is not available.
    pub note: Option<String>,
}

impl Choice {
    fn new(label: impl Into<String>, sets: Vec<(&'static str, String)>) -> Self {
        Self {
            label: label.into(),
            identify: sets.len(),
            sets,
            available: true,
            note: None,
        }
    }

    fn note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    fn available(mut self, available: bool, why_not: &str) -> Self {
        self.available = available;
        if !available {
            self.note = Some(why_not.to_owned());
        }
        self
    }
}

/// A setting's row.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    /// Its name on the screen.
    pub label: &'static str,
    /// What it does, for the description line.
    pub help: String,
    /// Its choices, in order.
    pub choices: Vec<Choice>,
    /// Why it cannot be changed now, if it can't: drawn greyed, this in the description.
    pub off: Option<String>,
    /// What it shows while greyed, when not its value (the monitor's own size).
    pub shown_off: Option<String>,
}

/// What a row does when taken, besides changing a setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    /// The HDR calibration pages (#125).
    Calibrate,
}

/// A row of a tab.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A section's title.
    Header(&'static str),
    /// A setting and its choices.
    Setting(Setting),
    /// A button among the rows.
    Action {
        /// Its words.
        label: &'static str,
        /// What it does.
        help: String,
        /// Why it cannot be taken now, if it can't.
        off: Option<String>,
        /// What it does.
        action: RowAction,
    },
}

impl Row {
    /// Whether the cursor stops on it.
    pub fn focusable(&self) -> bool {
        !matches!(self, Self::Header(_))
    }
}

/// The changes made on the settings screen and not applied yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    /// Values chosen, by name.
    values: BTreeMap<String, String>,
    /// Names put back to their defaults (the Defaults button): their lines leave the user's file.
    defaults: BTreeSet<String>,
}

impl Draft {
    /// The value `name` shows on the screen: the draft's, else its default when Defaults was
    /// taken, else for a setting read at start the user's choice for the next run, else its
    /// value now.
    pub fn value(&self, settings: &Settings, name: &str) -> Option<String> {
        if let Some(v) = self.values.get(name) {
            return Some(v.clone());
        }
        if self.defaults.contains(name) {
            return settings.default_text(name);
        }
        applied(settings, name)
    }

    /// Sets `name` to `value` in the draft; a value equal to the one in force leaves the draft
    /// where it is the user's already (or the default). One that Alt+Enter, a resize by hand,
    /// a key, `forge.cfg` or `--set` gave stays, for Apply to make it the user's.
    pub fn set(&mut self, settings: &Settings, name: &str, value: &str) {
        let value = settings
            .canonical(name, value)
            .unwrap_or_else(|| value.to_owned());
        self.defaults.remove(name);
        let users = settings.info(name).is_some_and(|v| {
            matches!(v.source, Source::Default | Source::User)
                || v.flags.contains(Flags::RESTART) && settings.user_value(name).is_some()
        });
        if users && applied(settings, name).as_deref() == Some(value.as_str()) {
            self.values.remove(name);
        } else {
            self.values.insert(name.to_owned(), value);
        }
    }

    /// Puts `name` back to its default (the Defaults button).
    pub fn reset(&mut self, name: &str) {
        self.values.remove(name);
        self.defaults.insert(name.to_owned());
    }

    /// Whether applying it would change anything.
    pub fn is_dirty(&self, settings: &Settings) -> bool {
        !self.values.is_empty() || self.defaults.iter().any(|n| self.resets(settings, n))
    }

    /// Whether `name` would change if applied.
    pub fn changes(&self, settings: &Settings, name: &str) -> bool {
        self.values.contains_key(name)
            || self.defaults.contains(name) && self.resets(settings, name)
    }

    /// Whether putting `name` back to its default changes anything: a line in the user's file, or
    /// a value not at its default.
    fn resets(&self, settings: &Settings, name: &str) -> bool {
        settings.user_value(name).is_some()
            || applied(settings, name) != settings.default_text(name)
    }

    /// Applies it: the user's choices written as the user's ([`Source::User`]), those read at
    /// start ([`Flags::RESTART`]) kept for the next run only. Returns the names that apply after
    /// a restart.
    pub fn apply(&mut self, settings: &mut Settings) -> Vec<String> {
        let mut later = Vec::new();
        for name in std::mem::take(&mut self.defaults) {
            if restart(settings, &name) && settings.user_value(&name).is_some() {
                later.push(name.clone());
            }
            settings.forget_user(&name);
        }
        for (name, value) in std::mem::take(&mut self.values) {
            let result = if restart(settings, &name) {
                later.push(name.clone());
                settings.set_user_next_run(&name, &value)
            } else {
                settings.set_text(&name, &value, Source::User)
            };
            if let Err(e) = result {
                tracing::warn!("the menu's {name}: {e}");
            }
        }
        later
    }

    /// Forgets every change.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// `name`'s value in force for the screen: for a setting read at start, the user's choice for the
/// next run where there is one.
fn applied(settings: &Settings, name: &str) -> Option<String> {
    let info = settings.info(name)?;
    if info.flags.contains(Flags::RESTART)
        && let Some(v) = settings.user_value(name)
    {
        return Some(v.to_owned());
    }
    Some(info.value_text())
}

/// Whether `name` is read at start.
pub fn restart(settings: &Settings, name: &str) -> bool {
    settings
        .info(name)
        .is_some_and(|v| v.flags.contains(Flags::RESTART))
}

impl Setting {
    /// The choice in force (`value` gives each setting's value as the screen shows it), if one is.
    pub fn current(&self, value: &dyn Fn(&str) -> Option<String>) -> Option<usize> {
        self.choices.iter().position(|c| {
            c.sets[..c.identify]
                .iter()
                .all(|(name, v)| value(name).as_deref() == Some(v.as_str()))
        })
    }

    /// The settings it sets, each once.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .choices
            .iter()
            .flat_map(|c| c.sets.iter().map(|(n, _)| *n))
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// The choice `dir` steps from the one in force, among those available: kept at the ends
    /// unless `wrap`; from no choice in force, the first (or the last, stepping back).
    pub fn step(&self, current: Option<usize>, dir: i64, wrap: bool) -> Option<usize> {
        let available: Vec<usize> = (0..self.choices.len())
            .filter(|&i| self.choices[i].available)
            .collect();
        let (first, last) = (*available.first()?, *available.last()?);
        let Some(now) = current else {
            return Some(if dir < 0 { last } else { first });
        };
        let next = if dir < 0 {
            available.iter().rev().find(|&&i| i < now).copied()
        } else {
            available.iter().find(|&&i| i > now).copied()
        };
        match next {
            Some(i) => Some(i),
            None if wrap => Some(if dir < 0 { last } else { first }),
            None => Some(now),
        }
    }
}

/// `choices`' indices as `(label, value)` pairs of one setting.
fn plain(name: &'static str, choices: &[(&str, &str)]) -> Vec<Choice> {
    choices
        .iter()
        .map(|&(label, value)| Choice::new(label, vec![(name, value.to_owned())]))
        .collect()
}

/// Off and on, for a switch.
fn switch(name: &'static str) -> Vec<Choice> {
    plain(name, &[("Off", "0"), ("On", "1")])
}

/// A row, if its first setting is registered.
fn setting(
    settings: &Settings,
    name: &str,
    label: &'static str,
    help: &str,
    choices: Vec<Choice>,
) -> Option<Row> {
    settings.info(name)?;
    Some(Row::Setting(Setting {
        label,
        help: help.to_owned(),
        choices,
        off: None,
        shown_off: None,
    }))
}

/// `[w, h]` divided by `ratio`, as a render size.
fn render_size(window: [u32; 2], ratio: f32) -> String {
    let w = (window[0] as f32 / ratio).round() as u32;
    let h = (window[1] as f32 / ratio).round() as u32;
    format!("{w} x {h}")
}

/// The rows of `tab` for the settings registered in `settings`, on this machine (`caps`), the
/// draft's values shown (`value`).
pub fn rows(
    tab: Tab,
    settings: &Settings,
    caps: &Caps,
    value: &dyn Fn(&str) -> Option<String>,
) -> Vec<Row> {
    let rows = match tab {
        Tab::Graphics => graphics(settings, caps, value),
        Tab::Display => display(settings, caps, value),
        Tab::Game => game(settings),
    };
    // A section's title with nothing under it goes.
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    for row in rows {
        if matches!(row, Row::Header(_)) && matches!(out.last(), Some(Row::Header(_))) {
            out.pop();
        }
        out.push(row);
    }
    if matches!(out.last(), Some(Row::Header(_))) {
        out.pop();
    }
    out
}

fn graphics(settings: &Settings, caps: &Caps, value: &dyn Fn(&str) -> Option<String>) -> Vec<Row> {
    let mut rows = vec![Row::Header("IMAGE")];
    // The preset: chosen by `r.preset`, setting its patch's names this demo registers.
    let preset = presets::PRESETS
        .iter()
        .map(|&p| {
            let mut sets = vec![("r.preset", p.to_owned())];
            sets.extend(
                presets::patch(p)
                    .iter()
                    .filter(|(n, _)| settings.info(n).is_some())
                    .filter_map(|&(n, v)| settings.canonical(n, v).map(|v| (n, v))),
            );
            let mut choice = Choice::new(capitalised(p), sets);
            choice.identify = 1;
            if p == "custom" {
                choice = choice.note("Custom: the rows below as they are set one by one.");
            }
            choice
        })
        .collect();
    rows.extend(setting(
        settings,
        "r.preset",
        "Quality preset",
        "Sets the rows below together, from Low to Ultra; changing one of them makes it Custom. \
         The first launch picked one from the GPU's memory.",
        preset,
    ));
    rows.extend(anti_aliasing(settings, caps));
    if let Some(Row::Setting(mut s)) = setting(
        settings,
        "r.fsr.sharpen",
        "FSR sharpening",
        "Sharpens FSR's picture (AMD's RCAS): more is crisper, with more shimmer on fine detail.",
        std::iter::once(Choice::new("Off", vec![("r.fsr.sharpen", "0".to_owned())]))
            .chain([0.25, 0.5, 0.75, 1.0].map(|k| {
                Choice::new(
                    format!("{:.0} %", k * 100.0),
                    vec![
                        ("r.fsr.sharpen", "1".to_owned()),
                        ("r.fsr.sharpness", fmt_number(k)),
                    ],
                )
            }))
            .collect(),
    ) {
        if value("r.aa").as_deref() != Some("fsr") {
            s.off = Some("Only with FSR: choose an FSR row above first.".to_owned());
        }
        rows.push(Row::Setting(s));
    }
    rows.push(Row::Header("LIGHTING"));
    rows.extend(setting(
        settings,
        "r.shadows",
        "Sun shadows",
        "The sun's shadows, traced by rays against the scene.",
        switch("r.shadows"),
    ));
    let mut gi = vec![
        Choice::new("Probes", vec![("r.gi.trace", "0".to_owned())])
            .note("Probes: the light of a grid of probes, the cheapest."),
    ];
    if settings.info("r.gi.cache").is_some() {
        gi.push(
            Choice::new(
                "Traced",
                vec![
                    ("r.gi.trace", "1".to_owned()),
                    ("r.gi.cache", "off".to_owned()),
                ],
            )
            .note(
                "Traced: a ray a pixel to the scene, the probes' light where it lands, denoised.",
            ),
        );
        gi.push(
            Choice::new(
                "Traced, SHaRC indoors",
                vec![
                    ("r.gi.trace", "1".to_owned()),
                    ("r.gi.cache", "auto".to_owned()),
                ],
            )
            .note(
                "Traced, and in rooms the light NVIDIA's SHaRC gathers on the surfaces round you \
                 (D-090): nearer the true light indoors.",
            ),
        );
    } else {
        gi.push(Choice::new("Traced", vec![("r.gi.trace", "1".to_owned())]));
    }
    rows.extend(setting(
        settings,
        "r.gi.trace",
        "Global illumination",
        "The light bounced between surfaces.",
        gi,
    ));
    rows.extend(setting(
        settings,
        "r.ao",
        "Ambient occlusion",
        "Darkens the sky's light in corners and creases (GTAO).",
        switch("r.ao"),
    ));
    rows.extend(setting(
        settings,
        "r.reflections.rays",
        "Ray-traced reflections",
        "Mirror rays in smooth surfaces: glass, metal and water show the scene round them.",
        switch("r.reflections.rays"),
    ));
    rows.extend(setting(
        settings,
        "r.exposure.local",
        "Local exposure",
        "Lifts a dark room seen beside a sunlit doorway, as the eye does (D-067).",
        plain("r.exposure.local", &[("Off", "off"), ("On", "on")]),
    ));
    rows.extend(setting(
        settings,
        "r.bloom",
        "Bloom",
        "The glow round bright lights.",
        switch("r.bloom"),
    ));
    rows.push(Row::Header("DETAIL"));
    rows.extend(setting(
        settings,
        "r.lod.error",
        "Level of detail",
        "How far a mesh may stray from its finest shape on screen before a finer one is drawn: \
         smaller is sharper and slower.",
        plain(
            "r.lod.error",
            &[
                ("Ultra (0.5 px)", "0.5"),
                ("High (1 px)", "1"),
                ("Medium (2 px)", "2"),
                ("Low (3 px)", "3"),
            ],
        ),
    ));
    rows
}

/// The anti-aliasing and upscaling row: `r.aa`, and the upscalers' modes where the demo renders
/// smaller (it registers `r.dlss.mode` and `r.fsr.mode`).
fn anti_aliasing(settings: &Settings, caps: &Caps) -> Option<Row> {
    settings.info("r.aa")?;
    const NO_DLSS: &str = "DLSS needs an NVIDIA RTX GPU and NVIDIA's Streamline, loaded at start.";
    let window = caps.window;
    let size = format!("{} x {}", window[0], window[1]);
    let mut choices = vec![
        Choice::new("Off", vec![("r.aa", "off".to_owned())])
            .note("No anti-aliasing: edges step and shimmer."),
        Choice::new("DLAA", vec![("r.aa", "dlaa".to_owned())])
            .note(format!(
                "NVIDIA DLAA: DLSS's anti-aliasing at the window's size ({size}), the reference image."
            ))
            .available(caps.dlss, NO_DLSS),
    ];
    let dlss_modes = settings.info("r.dlss.mode").is_some();
    if dlss_modes {
        for (mode, label, ratio) in [
            ("quality", "DLSS Quality", 1.5),
            ("balanced", "DLSS Balanced", 1.72),
            ("performance", "DLSS Performance", 2.0),
            ("ultra-performance", "DLSS Ultra Performance", 3.0),
        ] {
            choices.push(
                Choice::new(
                    label,
                    vec![
                        ("r.aa", "dlss".to_owned()),
                        ("r.dlss.mode", mode.to_owned()),
                    ],
                )
                .note(format!(
                    "NVIDIA {label}: renders at about {} and rebuilds {size}.",
                    render_size(window, ratio)
                ))
                .available(caps.dlss, NO_DLSS),
            );
        }
    }
    let fsr_modes = settings.info("r.fsr.mode").is_some();
    let mut native = vec![("r.aa", "fsr".to_owned())];
    if fsr_modes {
        native.push(("r.fsr.mode", "native".to_owned()));
    }
    choices.push(Choice::new("FSR Native AA", native).note(format!(
        "AMD FSR at the window's size ({size}): its anti-aliasing without upscaling, on any GPU."
    )));
    if fsr_modes {
        for (mode, label, ratio) in [
            ("quality", "FSR Quality", 1.5),
            ("balanced", "FSR Balanced", 1.7),
            ("performance", "FSR Performance", 2.0),
            ("ultra-performance", "FSR Ultra Performance", 3.0),
        ] {
            choices.push(
                Choice::new(
                    label,
                    vec![("r.aa", "fsr".to_owned()), ("r.fsr.mode", mode.to_owned())],
                )
                .note(format!(
                    "AMD {label}: renders at {} and rebuilds {size}, on any GPU.",
                    render_size(window, ratio)
                )),
            );
        }
    }
    let help = if dlss_modes || fsr_modes {
        "How edges are smoothed, and whether the frame renders smaller and is rebuilt at the \
         window's size (faster)."
    } else {
        "How edges are smoothed. This scene draws at the window's size: its upscaling modes are \
         not built yet."
    };
    setting(
        settings,
        "r.aa",
        "Anti-aliasing and upscaling",
        help,
        choices,
    )
}

fn display(settings: &Settings, caps: &Caps, value: &dyn Fn(&str) -> Option<String>) -> Vec<Row> {
    let mut rows = vec![Row::Header("WINDOW")];
    let fullscreen = value("app.window").as_deref() == Some("fullscreen");
    rows.extend(setting(
        settings,
        "app.window",
        "Window mode",
        "A window, or borderless fullscreen on the monitor at its own size (Alt+Enter switches \
         too, for the run). Borderless presents as directly as an exclusive mode would.",
        plain(
            "app.window",
            &[
                ("Windowed", "windowed"),
                ("Borderless fullscreen", "fullscreen"),
            ],
        ),
    ));
    if settings.info("app.window.width").is_some() {
        let monitor = caps.monitor_size();
        let mut sizes: Vec<[u32; 2]> = [
            [1280, 720],
            [1600, 900],
            [1920, 1080],
            [2560, 1440],
            [3200, 1800],
            [3840, 2160],
        ]
        .into_iter()
        .filter(|s| monitor.is_none_or(|m| s[0] <= m[0] && s[1] <= m[1]))
        .collect();
        // The size in force too, when it is not a usual one (a window resized by hand).
        let now = [value("app.window.width"), value("app.window.height")]
            .map(|v| v.and_then(|v| v.parse::<u32>().ok()).unwrap_or(0));
        if now.iter().all(|&v| v > 0) && !sizes.contains(&now) {
            sizes.push(now);
            sizes.sort_unstable_by_key(|s| s[0] * s[1]);
        }
        let choices = sizes
            .iter()
            .map(|s| {
                Choice::new(
                    format!("{} x {}", s[0], s[1]),
                    vec![
                        ("app.window.width", s[0].to_string()),
                        ("app.window.height", s[1].to_string()),
                    ],
                )
            })
            .collect();
        let mut row = Setting {
            label: "Resolution",
            help: "The window's size, pixels.".to_owned(),
            choices,
            off: None,
            shown_off: None,
        };
        if fullscreen {
            let size = monitor.map_or("its".to_owned(), |m| format!("{} x {}", m[0], m[1]));
            row.off = Some(format!(
                "Borderless fullscreen takes the monitor's own size, {size}: the upscaling modes \
                 render smaller."
            ));
            row.shown_off = monitor.map(|m| format!("{} x {}", m[0], m[1]));
        }
        rows.push(Row::Setting(row));
    }
    if settings.info("app.monitor").is_some() {
        let mut choices = vec![Choice::new("Automatic", vec![("app.monitor", "0".to_owned())])
            .note("Automatic: the second monitor where there is one (FORGE_MONITOR says otherwise), else the main one.")];
        for (i, m) in caps.monitors.iter().enumerate() {
            let main = if m.primary { ", main" } else { "" };
            choices.push(Choice::new(
                format!("{}: {} x {}{main}", i + 1, m.size[0], m.size[1]),
                vec![("app.monitor", (i + 1).to_string())],
            ));
        }
        rows.push(Row::Setting(Setting {
            label: "Monitor",
            help: "The monitor the game shows on.".to_owned(),
            choices,
            off: (caps.monitors.len() < 2).then(|| "One monitor.".to_owned()),
            shown_off: None,
        }));
    }
    rows.push(Row::Header("FRAMES"));
    rows.extend(setting(
        settings,
        "app.vsync",
        "VSync",
        "Waits for the display's refresh: never more frames than it shows, a steady pace. Off, \
         frames show as they come, without tearing.",
        switch("app.vsync"),
    ));
    rows.extend(setting(
        settings,
        "app.fps_max",
        "Frame rate limit",
        "The most frames a second the game draws, the rest of each frame left to sleep: a cooler, \
         quieter GPU. Off draws as fast as it can.",
        plain(
            "app.fps_max",
            &[
                ("Off", "0"),
                ("30", "30"),
                ("60", "60"),
                ("90", "90"),
                ("120", "120"),
                ("144", "144"),
                ("165", "165"),
                ("240", "240"),
            ],
        ),
    ));
    rows.push(Row::Header("PICTURE"));
    if let Some(Row::Setting(mut row)) = setting(
        settings,
        "app.hdr.mode",
        "HDR",
        "The picture's range on an HDR display: brighter lights, deeper shadows.",
        plain(
            "app.hdr.mode",
            &[("Off", "off"), ("HDR10", "hdr10"), ("scRGB", "scrgb")],
        ),
    ) {
        if !caps.hdr_display {
            row.off = Some(
                "Windows shows this display in SDR: turn on Use HDR in Windows' display settings, \
                 then come back."
                    .to_owned(),
            );
        }
        rows.push(Row::Setting(row));
        rows.push(Row::Action {
            label: "HDR calibration",
            help: "The display's peak, black and white, on three pages (#125).".to_owned(),
            off: (!caps.hdr_output).then(|| "Only with HDR on: apply HDR first.".to_owned()),
            action: RowAction::Calibrate,
        });
    }
    if settings.info("r.brightness").is_some() {
        let choices = (-8..=8)
            .map(|k| {
                let stops = f64::from(k) * 0.25;
                let label = if k == 0 {
                    "Default".to_owned()
                } else {
                    format!("{stops:+.2}")
                };
                Choice::new(label, vec![("r.brightness", fmt_number(stops))])
            })
            .collect();
        rows.extend(setting(
            settings,
            "r.brightness",
            "Brightness",
            "Brightens or darkens the picture, in stops of exposure over the scene's own (0).",
            choices,
        ));
    }
    rows
}

fn game(settings: &Settings) -> Vec<Row> {
    let level = |name: &'static str| -> Vec<Choice> {
        (0..=10)
            .map(|k| {
                Choice::new(
                    format!("{} %", k * 10),
                    vec![(name, fmt_number(f64::from(k) / 10.0))],
                )
            })
            .collect()
    };
    let mut rows = vec![Row::Header("SOUND")];
    rows.extend(setting(
        settings,
        "snd.volume",
        "Volume",
        "The sound's master level.",
        level("snd.volume"),
    ));
    rows.extend(setting(
        settings,
        "snd.ambience",
        "Ambience",
        "The wind, the birds, the cicadas and the sea, under the master level.",
        level("snd.ambience"),
    ));
    rows.push(Row::Header("CONTROLS"));
    rows.extend(setting(
        settings,
        "app.mouse_look",
        "Mouse look",
        "The mouse turns the view without a button held, the cursor hidden.",
        switch("app.mouse_look"),
    ));
    rows.extend(setting(
        settings,
        "r.display_info",
        "Frame info",
        "Frame times and statistics over the picture (F1 too).",
        plain(
            "r.display_info",
            &[("Off", "0"), ("On", "1"), ("Verbose", "2")],
        ),
    ));
    rows
}

/// A number as the settings write it: no trailing zeros.
fn fmt_number(v: f64) -> String {
    let text = format!("{v:.4}");
    let text = text.trim_end_matches('0');
    text.strip_suffix('.').unwrap_or(text).to_owned()
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::settings::Spec;

    fn registry(modes: bool) -> Settings {
        let mut s = Settings::new();
        s.add(Spec::choice("r.preset", presets::PRESETS, 0, "preset"));
        s.add(Spec::choice(
            "r.aa",
            &["off", "dlaa", "dlss", "fsr"],
            1,
            "aa",
        ));
        if modes {
            s.add(Spec::choice(
                "r.dlss.mode",
                &["quality", "balanced", "performance", "ultra-performance"],
                0,
                "dlss",
            ));
            s.add(Spec::choice(
                "r.fsr.mode",
                &[
                    "native",
                    "quality",
                    "balanced",
                    "performance",
                    "ultra-performance",
                ],
                0,
                "fsr",
            ));
        }
        s.add(Spec::bool("r.ao", true, "ao"));
        s.add(Spec::bool("r.reflections.rays", true, "rays"));
        s.add(Spec::float("r.lod.error", 1.0, 0.125..=16.0, "lod"));
        s.add(Spec::choice(
            "app.window",
            &["windowed", "fullscreen"],
            0,
            "window",
        ));
        s
    }

    fn caps(dlss: bool) -> Caps {
        Caps {
            dlss,
            monitors: vec![
                Monitor {
                    size: [2560, 1440],
                    primary: true,
                },
                Monitor {
                    size: [1920, 1080],
                    primary: false,
                },
            ],
            monitor: Some(1),
            hdr_display: false,
            hdr_output: false,
            window: [1920, 1080],
        }
    }

    fn row<'a>(rows: &'a [Row], label: &str) -> &'a Setting {
        rows.iter()
            .find_map(|r| match r {
                Row::Setting(s) if s.label == label => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no row {label}"))
    }

    fn labels(s: &Setting) -> Vec<(&str, bool)> {
        s.choices
            .iter()
            .map(|c| (c.label.as_str(), c.available))
            .collect()
    }

    #[test]
    fn the_anti_aliasing_row_offers_what_the_gpu_and_the_demo_have() {
        let s = registry(true);
        let draft = Draft::default();
        let value = |n: &str| draft.value(&s, n);
        let rows = rows(Tab::Graphics, &s, &caps(true), &value);
        let aa = row(&rows, "Anti-aliasing and upscaling");
        assert_eq!(aa.choices.len(), 11);
        assert!(aa.choices.iter().all(|c| c.available));
        assert_eq!(aa.current(&value), Some(1), "DLAA by default");
        let quality = &aa.choices[2];
        assert_eq!(quality.label, "DLSS Quality");
        assert_eq!(
            quality.note.as_deref(),
            Some("NVIDIA DLSS Quality: renders at about 1280 x 720 and rebuilds 1920 x 1080.")
        );
        // No DLSS on this GPU: its rows stay, greyed, and stepping skips them.
        let rows = super::rows(Tab::Graphics, &s, &caps(false), &value);
        let aa = row(&rows, "Anti-aliasing and upscaling");
        assert_eq!(
            labels(aa)[..3],
            [("Off", true), ("DLAA", false), ("DLSS Quality", false)]
        );
        assert_eq!(aa.step(Some(0), 1, false), Some(6));
        assert_eq!(aa.choices[6].label, "FSR Native AA");
        assert_eq!(aa.step(Some(6), -1, false), Some(0));
        // A demo that draws at the window's size: no upscaling modes.
        let s = registry(false);
        let value = |n: &str| draft.value(&s, n);
        let rows = super::rows(Tab::Graphics, &s, &caps(true), &value);
        let aa = row(&rows, "Anti-aliasing and upscaling");
        assert_eq!(
            labels(aa),
            [("Off", true), ("DLAA", true), ("FSR Native AA", true)]
        );
        assert!(aa.help.contains("not built yet"));
    }

    #[test]
    fn stepping_keeps_to_the_ends_unless_it_wraps() {
        let s = Setting {
            label: "x",
            help: String::new(),
            choices: plain("x", &[("a", "0"), ("b", "1"), ("c", "2")]),
            off: None,
            shown_off: None,
        };
        assert_eq!(s.step(Some(2), 1, false), Some(2));
        assert_eq!(s.step(Some(2), 1, true), Some(0));
        assert_eq!(s.step(Some(0), -1, true), Some(2));
        assert_eq!(s.step(None, 1, false), Some(0));
        assert_eq!(s.step(None, -1, false), Some(2));
    }

    #[test]
    fn a_draft_waits_for_apply_and_shows_its_values_meanwhile() {
        let mut s = registry(true);
        let mut draft = Draft::default();
        draft.set(&s, "r.ao", "off");
        assert_eq!(draft.value(&s, "r.ao").as_deref(), Some("0"));
        assert_eq!(s.info("r.ao").map(|v| v.value_text()).as_deref(), Some("1"));
        assert!(draft.is_dirty(&s));
        assert!(draft.changes(&s, "r.ao"));
        // Back to the value in force: nothing to apply.
        draft.set(&s, "r.ao", "on");
        assert!(!draft.is_dirty(&s));
        draft.set(&s, "r.ao", "0");
        draft.set(&s, "r.aa", "fsr");
        assert!(draft.apply(&mut s).is_empty());
        assert!(!draft.is_dirty(&s));
        assert_eq!(s.user_lines(), ["r.aa fsr", "r.ao 0"]);
        assert_eq!(s.info("r.ao").map(|v| v.source), Some(Source::User));
        // Defaults: the lines go, the values come back.
        for name in ["r.aa", "r.ao", "r.lod.error"] {
            draft.reset(name);
        }
        assert!(draft.is_dirty(&s));
        assert!(!draft.changes(&s, "r.lod.error"));
        assert_eq!(draft.value(&s, "r.aa").as_deref(), Some("dlaa"));
        draft.apply(&mut s);
        assert!(s.user_lines().is_empty());
        assert_eq!(
            s.info("r.aa").map(|v| v.value_text()).as_deref(),
            Some("dlaa")
        );
    }

    #[test]
    fn a_setting_read_at_start_is_applied_for_the_next_run() {
        let mut s = registry(true);
        s.add(Spec::choice("app.gpu", &["auto", "nvidia", "amd"], 0, "gpu").restart());
        let mut draft = Draft::default();
        draft.set(&s, "app.gpu", "amd");
        draft.set(&s, "r.ao", "0");
        assert_eq!(draft.apply(&mut s), ["app.gpu"]);
        // Now as before; the screen shows the next run's.
        assert_eq!(
            s.info("app.gpu").map(|v| v.value_text()).as_deref(),
            Some("auto")
        );
        assert_eq!(draft.value(&s, "app.gpu").as_deref(), Some("amd"));
        assert!(restart(&s, "app.gpu") && !restart(&s, "r.ao"));
        assert_eq!(s.user_lines(), ["app.gpu amd", "r.ao 0"]);
    }

    #[test]
    fn the_preset_row_sets_its_patch_and_is_known_by_its_name() {
        let s = registry(false);
        let draft = Draft::default();
        let value = |n: &str| draft.value(&s, n);
        let rows = rows(Tab::Graphics, &s, &caps(true), &value);
        let preset = row(&rows, "Quality preset");
        let low = &preset.choices[1];
        assert_eq!(low.label, "Low");
        assert_eq!(low.identify, 1);
        // The village registers neither upscaler's mode: only its names are set.
        assert_eq!(
            low.sets,
            [
                ("r.preset", "low".to_owned()),
                ("r.ao", "0".to_owned()),
                ("r.reflections.rays", "0".to_owned()),
                ("r.lod.error", "3".to_owned()),
            ]
        );
        assert_eq!(preset.current(&value), Some(0));
    }

    #[test]
    fn display_rows_follow_the_window_mode_and_the_display() {
        let mut s = registry(false);
        s.add(Spec::int("app.window.width", 1600, 0..=16384, "w"));
        s.add(Spec::int("app.window.height", 900, 0..=16384, "h"));
        s.add(Spec::int("app.monitor", 0, 0..=16, "monitor"));
        s.add(Spec::choice(
            "app.hdr.mode",
            &["off", "hdr10", "scrgb", "offscreen"],
            0,
            "hdr",
        ));
        let mut draft = Draft::default();
        let c = caps(true);
        {
            let value = |n: &str| draft.value(&s, n);
            let rows = rows(Tab::Display, &s, &c, &value);
            let res = row(&rows, "Resolution");
            // Sizes that fit the window's monitor (1920 x 1080).
            assert_eq!(
                labels(res).iter().map(|l| l.0).collect::<Vec<_>>(),
                ["1280 x 720", "1600 x 900", "1920 x 1080"]
            );
            assert_eq!(res.current(&value), Some(1));
            assert!(res.off.is_none());
            assert!(row(&rows, "HDR").off.is_some());
            assert_eq!(row(&rows, "Monitor").choices[2].label, "2: 1920 x 1080");
        }
        draft.set(&s, "app.window", "fullscreen");
        let value = |n: &str| draft.value(&s, n);
        let rows = rows(Tab::Display, &s, &c, &value);
        let res = row(&rows, "Resolution");
        assert!(res.off.is_some());
        assert_eq!(res.shown_off.as_deref(), Some("1920 x 1080"));
    }
}

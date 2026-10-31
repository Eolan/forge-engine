//! The game's menu on Esc (#352, the owner on 2026-10-22): resume, settings and quit, over the
//! picture as the HUD's quads (#328), with glyphs of its own, which [`crate::Context::hud_text`]
//! lends to the games too.
//!
//! - **Esc** opens it, and on its first page closes it again (resume); on the settings screen it
//!   goes back, first asking whether to apply changes not applied yet.
//! - **The settings screen** (#377, the owner on 2026-10-31: "a game menu that looks more
//!   professional"): tabs for the graphics, the display, and the sound and controls
//!   ([`items`]). Each row shows its value and steps through its choices; a line under the rows
//!   says what the highlighted one does and whether it applies at once or after a restart.
//!   Changes wait in a draft ([`items::Draft`]) for Apply, which sets them as the user's
//!   (`Source::User`, written to `settings/user.cfg`); Defaults puts the tab's back; Back leaves.
//! - **The keys,** by their place on the keyboard (any layout): Up and Down (or W and S) choose,
//!   Left and Right (or A and D) change, Enter or Space takes, Tab (or E, Page Down) and Q (or
//!   Page Up) turn the tabs. The mouse chooses by hovering; a left click takes or steps forward,
//!   a right click steps back.
//! - **A solo game pauses behind it** ([`crate::Context::paused`]); a networked one
//!   ([`crate::Demo::networked`]) runs on. The shell draws the game behind it as frosted glass
//!   (the owner: "a glass gaussian transparency effect but almost opaque", "full screen";
//!   [`crate::glass`]).
//! - **Two looks** (`app.menu_theme`, [`Theme`]): Carbon, the default, the owner's streaming
//!   overlays' ([`carbon`], 2026-10-31: "with the theme of the project in the directory
//!   obs-streaming-project"), and the first one, plainer ([`base`], the owner: "keep the one you
//!   did before somewhere, it fits more the base theme, but keep carbon as the current theme").
//!   They share the menu's state and keys; each lays out and draws its own column.

mod base;
mod carbon;
pub mod items;

use std::path::Path;
use std::sync::Arc;

use forge_core::settings::{Settings, Value};
use forge_gpu::{Device, GraphImage, ImageDesc, Result, vk};
use winit::keyboard::KeyCode;

use crate::hud::HudQuad;
use crate::overlay::{FontAtlas, GLYPHS, rasterize_builtin, rasterize_ttf};
use items::{Caps, Draft, Row, RowAction, Setting, Tab};

/// The main page's entries.
const MAIN: [&str; 3] = ["Resume", "Settings", "Quit"];
/// The settings screen's buttons, under its rows.
const BUTTONS: [&str; 3] = ["Apply", "Defaults", "Back"];
/// The unsaved-changes dialog's buttons.
const DIALOG: [&str; 3] = ["Apply", "Discard", "Cancel"];
/// The settings screen's keys, under its buttons.
const HINT: &str = "Up/Down choose  Left/Right change  Tab next page  Enter take  Esc back";
/// The column's width in characters of the body text.
const COLUMNS: f32 = 66.0;

/// Empty pixels round each glyph in an atlas, so the linear filter never reads a neighbour.
const PAD: u32 = 2;
/// Glyphs in a row of an atlas.
const ATLAS_COLUMNS: usize = 16;

/// The menu's looks, `app.menu_theme`'s choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// The owner's streaming overlays' ([`carbon`]): the default.
    #[default]
    Carbon,
    /// The first one, plainer ([`base`]).
    Base,
}

impl Theme {
    /// `app.menu_theme`'s choices, in its order.
    pub const NAMES: [&str; 2] = ["carbon", "base"];

    /// The one `settings` choose (Carbon where `app.menu_theme` is not registered).
    pub fn of(settings: &Settings) -> Self {
        match settings.info("app.menu_theme").map(|v| v.value) {
            Some(Value::Choice(1)) => Self::Base,
            _ => Self::Carbon,
        }
    }
}

/// What lies behind the menu, for its drawing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Behind {
    /// A solo game holds behind it.
    pub paused: bool,
    /// The shell drew the frosted glass under it; else the game is veiled here.
    pub glass: bool,
    /// The HUD blends on PQ's codes (HDR10) rather than in linear light.
    pub pq: bool,
}

/// What the menu asks of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// It closed: the game goes on.
    Resume,
    /// Close the window.
    Quit,
    /// It closed for the HDR calibration pages (#125).
    Calibrate,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Main,
    Settings,
}

/// Where the keys act on the settings screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    /// A row (its index among the tab's rows).
    Row(usize),
    /// One of [`BUTTONS`].
    Button(usize),
}

impl Default for Focus {
    fn default() -> Self {
        Self::Row(0)
    }
}

/// The menu's state.
#[derive(Debug, Default)]
pub struct Menu {
    open: bool,
    page: Page,
    /// The main page's chosen entry.
    selected: usize,
    tab: Tab,
    focus: Focus,
    /// The changes not applied yet.
    draft: Draft,
    /// The unsaved-changes dialog, and its chosen button.
    dialog: Option<usize>,
    /// A line after Apply or Defaults, until the next change.
    notice: Option<String>,
    /// What the shell knows of the machine, for the rows.
    caps: Caps,
}

/// The mono face's glyphs for each pixel of the text's size: their advance (the grid the
/// column's width is counted in) and their cells' height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    advance: f32,
    height: f32,
}

impl Metrics {
    /// How wide `text` is at `px` pixels in the mono face, one advance a letter.
    fn cells(self, px: f32, text: &str) -> f32 {
        self.advance * px * text.len() as f32
    }
}

/// The menu's typefaces (Carbon's, #377; the base look sets everything in the mono one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// JetBrains Mono, the overlay's font: the values, the sections' names, the hints; the
    /// games' HUD text.
    Mono,
    /// Rajdhani Medium: the rows' labels and the descriptions.
    Text,
    /// Rajdhani SemiBold: the tabs, the buttons, the main page's entries.
    Strong,
    /// Orbitron ExtraBold: the titles.
    Title,
}

impl Face {
    const ALL: [Face; 4] = [Face::Mono, Face::Text, Face::Strong, Face::Title];

    /// Its file under `assets/fonts` (the mono one is the overlay's font, wherever it is).
    fn file(self) -> Option<&'static str> {
        match self {
            Face::Mono => None,
            Face::Text => Some("rajdhani/Rajdhani-Medium.ttf"),
            Face::Strong => Some("rajdhani/Rajdhani-SemiBold.ttf"),
            Face::Title => Some("orbitron/Orbitron-ExtraBold.ttf"),
        }
    }

    /// The em sizes its glyphs are rasterised at, pixels: text takes the smallest at or above its
    /// size, so it is drawn at about 0.5 to 1.1 of an atlas; the titles reach twice the body
    /// text's size.
    fn sizes(self) -> &'static [f32] {
        match self {
            Face::Title => &[26.0, 56.0, 96.0],
            _ => &[26.0, 56.0],
        }
    }
}

/// How a line is set: its face, its size (pixels) and the space added between its letters
/// (ems).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    face: Face,
    px: f32,
    spacing: f32,
}

impl Style {
    const fn new(face: Face, px: f32, spacing: f32) -> Self {
        Self { face, px, spacing }
    }
}

/// How wide text is set: by the faces' own advances ([`MenuFont`]), or all of them on the mono
/// grid ([`Metrics`]: the tests, which have no font).
pub trait Measure {
    /// The mono face's metrics, which the layout is counted in.
    fn metrics(&self) -> Metrics;
    /// How wide `text` is set in `style`.
    fn width(&self, style: Style, text: &str) -> f32;
}

impl Measure for Metrics {
    fn metrics(&self) -> Metrics {
        *self
    }

    fn width(&self, style: Style, text: &str) -> f32 {
        let n = text.len() as f32;
        (self.advance * n + style.spacing * (n - 1.0).max(0.0)) * style.px
    }
}

/// Where everything goes, the window's pixels, as a theme lays it out: the rows', the tabs' and
/// the buttons' rectangles take the mouse; a line of text starts at its baseline (Carbon) or at
/// its cells' top (base).
#[derive(Debug, Default)]
struct Layout {
    /// The body text's size.
    px: f32,
    /// The column, top to bottom of the window.
    column: [f32; 4],
    /// Its inside, left and right, past the margins.
    inner: [f32; 2],
    /// The line over the title (Carbon's): where it starts and its size.
    call: ([f32; 2], f32),
    /// The title: where it starts and its size.
    title: ([f32; 2], f32),
    /// The fading line under the title (Carbon's).
    title_rule: [f32; 4],
    /// The tabs' rectangles (the settings screen).
    tabs: Vec<[f32; 4]>,
    /// The line under the tabs.
    rule: f32,
    /// Each row's rectangle: the main page's entries, or the tab's rows.
    rows: Vec<[f32; 4]>,
    /// The description's rectangle.
    description: [f32; 4],
    /// The buttons' rectangles.
    buttons: Vec<[f32; 4]>,
    /// The hint's line: its top (its capitals' in Carbon) and its size.
    hint: (f32, f32),
    /// The dialog's panel and its buttons.
    dialog: Option<([f32; 4], Vec<[f32; 4]>)>,
}

/// What lies under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Row(usize),
    Tab(usize),
    Button(usize),
    Dialog(usize),
}

fn inside(at: [f32; 2], r: [f32; 4]) -> bool {
    at[0] >= r[0] && at[0] < r[2] && at[1] >= r[1] && at[1] < r[3]
}

impl Menu {
    /// Whether it shows.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows it on its first page, the first entry chosen.
    pub fn open(&mut self) {
        let caps = std::mem::take(&mut self.caps);
        *self = Self {
            open: true,
            caps,
            ..Self::default()
        };
    }

    /// Shows the settings screen on `tab` (`app.menu graphics`: a capture of a page).
    pub fn open_settings(&mut self, tab: Tab) {
        self.open();
        self.page = Page::Settings;
        self.tab = tab;
    }

    /// Hides it; changes not applied are forgotten.
    pub fn close(&mut self) {
        self.open = false;
        self.draft.clear();
        self.dialog = None;
    }

    /// What the shell knows of the machine now: the rows follow it.
    pub fn set_caps(&mut self, caps: Caps) {
        self.caps = caps;
    }

    /// The tab's rows, the draft's values shown.
    fn rows(&self, settings: &Settings) -> Vec<Row> {
        items::rows(self.tab, settings, &self.caps, &|n| {
            self.draft.value(settings, n)
        })
    }

    /// A key went down (or repeats): what it asks of the shell.
    pub fn key(&mut self, code: KeyCode, settings: &mut Settings) -> Option<Action> {
        if let Some(choice) = self.dialog {
            match code {
                KeyCode::Escape => self.dialog = None,
                KeyCode::ArrowLeft | KeyCode::KeyA => self.dialog = Some(choice.saturating_sub(1)),
                KeyCode::ArrowRight | KeyCode::KeyD => self.dialog = Some((choice + 1).min(2)),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    self.dialog_take(choice, settings);
                }
                _ => {}
            }
            return None;
        }
        self.settle_focus(settings);
        match self.page {
            Page::Main => match code {
                KeyCode::Escape => {
                    self.close();
                    return Some(Action::Resume);
                }
                KeyCode::ArrowUp | KeyCode::KeyW => {
                    self.selected = (self.selected + MAIN.len() - 1) % MAIN.len();
                }
                KeyCode::ArrowDown | KeyCode::KeyS => {
                    self.selected = (self.selected + 1) % MAIN.len();
                }
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    return self.take_main(settings);
                }
                _ => {}
            },
            Page::Settings => match code {
                KeyCode::Escape => self.back(settings),
                KeyCode::ArrowUp | KeyCode::KeyW => self.move_focus(settings, -1),
                KeyCode::ArrowDown | KeyCode::KeyS => self.move_focus(settings, 1),
                KeyCode::ArrowLeft | KeyCode::KeyA => self.sideways(settings, -1),
                KeyCode::ArrowRight | KeyCode::KeyD => self.sideways(settings, 1),
                KeyCode::Tab | KeyCode::KeyE | KeyCode::PageDown => self.turn_tab(settings, 1),
                KeyCode::KeyQ | KeyCode::PageUp => self.turn_tab(settings, -1),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    return self.take(settings, 1);
                }
                _ => {}
            },
        }
        None
    }

    /// The cursor at `at` in a window of `size` pixels: what is under it is chosen.
    pub fn hover(&mut self, at: [f32; 2], size: [f32; 2], m: &dyn Measure, settings: &Settings) {
        self.settle_focus(settings);
        match self.under(at, size, m, settings) {
            Some(Hit::Row(i)) if self.page == Page::Main => self.selected = i,
            Some(Hit::Row(i)) => {
                if self.rows(settings).get(i).is_some_and(Row::focusable) {
                    self.focus = Focus::Row(i);
                }
            }
            Some(Hit::Button(b)) => self.focus = Focus::Button(b),
            Some(Hit::Dialog(b)) => self.dialog = Some(b),
            _ => {}
        }
    }

    /// A click at `at`: what is under it taken, a row stepped forward (`back`: a right click,
    /// back).
    pub fn click(
        &mut self,
        at: [f32; 2],
        size: [f32; 2],
        m: &dyn Measure,
        settings: &mut Settings,
        back: bool,
    ) -> Option<Action> {
        let hit = self.under(at, size, m, settings)?;
        self.hover(at, size, m, settings);
        // A right click only steps a setting back: on the main page or a button it does nothing.
        if back && (self.page == Page::Main || !matches!(hit, Hit::Row(_))) {
            return None;
        }
        match hit {
            Hit::Dialog(b) => {
                self.dialog_take(b, settings);
                None
            }
            _ if self.dialog.is_some() => None,
            Hit::Row(_) if self.page == Page::Main => self.take_main(settings),
            Hit::Tab(t) => {
                self.set_tab(settings, Tab::ALL[t]);
                None
            }
            // A section's title takes no click (the focus stayed where it was).
            Hit::Row(i) if self.focus != Focus::Row(i) => None,
            Hit::Row(_) | Hit::Button(_) => self.take(settings, if back { -1 } else { 1 }),
        }
    }

    /// Takes the main page's chosen entry.
    fn take_main(&mut self, settings: &Settings) -> Option<Action> {
        match self.selected {
            0 => {
                self.close();
                Some(Action::Resume)
            }
            1 => {
                self.page = Page::Settings;
                self.focus = Focus::Row(0);
                self.settle_focus(settings);
                None
            }
            _ => Some(Action::Quit),
        }
    }

    /// The focus as it acts on `rows`: on a row the cursor can stop on (the first one when it is
    /// not on one: a tab's first row is a section's title), or on a button.
    fn focused(&self, rows: &[Row]) -> Focus {
        match self.focus {
            Focus::Row(i) if !rows.get(i).is_some_and(Row::focusable) => rows
                .iter()
                .position(Row::focusable)
                .map_or(Focus::Button(0), Focus::Row),
            focus => focus,
        }
    }

    /// Keeps the focus where [`Menu::focused`] puts it.
    fn settle_focus(&mut self, settings: &Settings) {
        if self.page == Page::Settings {
            let rows = self.rows(settings);
            self.focus = self.focused(&rows);
        }
    }

    /// Takes what has the focus: a row steps by `dir` (round), a button acts.
    fn take(&mut self, settings: &mut Settings, dir: i64) -> Option<Action> {
        match self.focus {
            Focus::Row(i) => match self.rows(settings).get(i) {
                Some(Row::Setting(_)) => self.change(settings, dir, true),
                Some(Row::Action {
                    off: None,
                    action: RowAction::Calibrate,
                    ..
                }) if dir > 0 => {
                    // Its pages replace the menu: changes not applied would be lost unasked.
                    if self.draft.is_dirty(settings) {
                        self.notice = Some("Apply or discard the changes first.".to_owned());
                    } else {
                        self.close();
                        return Some(Action::Calibrate);
                    }
                }
                _ => {}
            },
            Focus::Button(0) => self.apply(settings),
            Focus::Button(1) => self.defaults(settings),
            Focus::Button(_) => self.back(settings),
        }
        None
    }

    /// Left or Right: the row's value stepped, or the next button.
    fn sideways(&mut self, settings: &mut Settings, dir: i64) {
        match self.focus {
            Focus::Row(_) => self.change(settings, dir, false),
            Focus::Button(b) => {
                self.focus = Focus::Button((b as i64 + dir).clamp(0, 2) as usize);
            }
        }
    }

    /// Up or Down: the focusable row before or after, past the last the buttons.
    fn move_focus(&mut self, settings: &Settings, dir: i64) {
        let rows = self.rows(settings);
        let focusable: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].focusable()).collect();
        self.focus = match (self.focus, dir < 0) {
            (Focus::Row(i), false) => focusable
                .iter()
                .find(|&&k| k > i)
                .map_or(Focus::Button(0), |&k| Focus::Row(k)),
            (Focus::Row(i), true) => focusable
                .iter()
                .rev()
                .find(|&&k| k < i)
                .map_or(Focus::Row(i), |&k| Focus::Row(k)),
            (Focus::Button(_), true) => focusable.last().map_or(self.focus, |&k| Focus::Row(k)),
            (button, false) => button,
        };
    }

    /// The next tab or the one before (round).
    fn turn_tab(&mut self, settings: &Settings, dir: i64) {
        let i = Tab::ALL.iter().position(|&t| t == self.tab).unwrap_or(0) as i64;
        let n = Tab::ALL.len() as i64;
        self.set_tab(settings, Tab::ALL[(i + dir).rem_euclid(n) as usize]);
    }

    fn set_tab(&mut self, settings: &Settings, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.focus = Focus::Row(0);
            self.settle_focus(settings);
        }
    }

    /// Steps the focused row by `dir` among the choices this machine can take (`wrap`: round),
    /// in the draft. A row the preset sets, changed, makes the preset custom.
    fn change(&mut self, settings: &Settings, dir: i64, wrap: bool) {
        let Focus::Row(i) = self.focus else {
            return;
        };
        let Some(Row::Setting(row)) = self.rows(settings).into_iter().nth(i) else {
            return;
        };
        if row.off.is_some() {
            return;
        }
        let current = row.current(&|n| self.draft.value(settings, n));
        let Some(next) = row.step(current, dir, wrap) else {
            return;
        };
        if Some(next) == current {
            return;
        }
        for (name, value) in &row.choices[next].sets {
            self.draft.set(settings, name, value);
        }
        if !row.names().contains(&"r.preset") {
            self.settle_preset(settings);
        }
        self.notice = None;
    }

    /// The draft's preset made custom once a setting it sets no longer has the preset's value.
    fn settle_preset(&mut self, settings: &Settings) {
        let Some(preset) = self.draft.value(settings, "r.preset") else {
            return;
        };
        let kept = crate::presets::patch(&preset).iter().all(|(name, value)| {
            settings.info(name).is_none()
                || self.draft.value(settings, name) == settings.canonical(name, value)
        });
        if !kept {
            self.draft.set(settings, "r.preset", "custom");
        }
    }

    /// Applies the draft.
    fn apply(&mut self, settings: &mut Settings) {
        if !self.draft.is_dirty(settings) {
            return;
        }
        let later = self.draft.apply(settings);
        self.notice = Some(if later.is_empty() {
            "Applied and saved.".to_owned()
        } else {
            "Saved: the rows marked restart apply after you restart.".to_owned()
        });
    }

    /// The tab's rows back to their defaults, in the draft.
    fn defaults(&mut self, settings: &Settings) {
        for row in self.rows(settings) {
            if let Row::Setting(s) = row {
                for name in s.names() {
                    self.draft.reset(name);
                }
            }
        }
        self.notice = Some("This page's defaults: Apply keeps them.".to_owned());
    }

    /// Back to the first page, asking first when the draft holds changes.
    fn back(&mut self, settings: &Settings) {
        if self.draft.is_dirty(settings) {
            self.dialog = Some(0);
            return;
        }
        self.draft.clear();
        self.notice = None;
        self.page = Page::Main;
        self.selected = 1;
    }

    /// The dialog's button `b` taken: apply and leave, discard and leave, or stay.
    fn dialog_take(&mut self, b: usize, settings: &mut Settings) {
        self.dialog = None;
        match b {
            0 => {
                self.draft.apply(settings);
                self.back(settings);
            }
            1 => {
                self.draft.clear();
                self.back(settings);
            }
            _ => {}
        }
    }

    /// What lies under `at`.
    fn under(
        &self,
        at: [f32; 2],
        size: [f32; 2],
        m: &dyn Measure,
        settings: &Settings,
    ) -> Option<Hit> {
        let l = self.layout(size, m, settings);
        if let Some((_, buttons)) = &l.dialog {
            return buttons.iter().position(|&r| inside(at, r)).map(Hit::Dialog);
        }
        if let Some(i) = l.rows.iter().position(|&r| inside(at, r)) {
            return Some(Hit::Row(i));
        }
        if let Some(t) = l.tabs.iter().position(|&r| inside(at, r)) {
            return Some(Hit::Tab(t));
        }
        l.buttons
            .iter()
            .position(|&r| inside(at, r))
            .map(Hit::Button)
    }

    /// The column as the theme lays it out.
    fn layout(&self, size: [f32; 2], measure: &dyn Measure, settings: &Settings) -> Layout {
        match Theme::of(settings) {
            Theme::Carbon => carbon::layout(self, size, measure, settings),
            Theme::Base => base::layout(self, size, measure.metrics(), settings),
        }
    }

    /// Its quads for a window of `size` pixels, over everything else, in the theme `settings`
    /// choose, over what is `behind` it.
    pub fn draw(
        &self,
        font: &MenuFont,
        size: [f32; 2],
        settings: &Settings,
        behind: Behind,
        out: &mut Vec<HudQuad>,
    ) {
        match Theme::of(settings) {
            Theme::Carbon => carbon::draw(self, font, size, settings, behind, out),
            Theme::Base => base::draw(self, font, size, settings, behind, out),
        }
    }

    /// The description of the focused row, and how it applies: at once, after a restart, or why
    /// it is greyed.
    fn describe(
        &self,
        settings: &Settings,
        rows: &[Row],
        focus: Focus,
    ) -> (String, Option<Status>) {
        match focus {
            Focus::Row(i) => match rows.get(i) {
                Some(Row::Setting(s)) => {
                    let value = |n: &str| self.draft.value(settings, n);
                    let note = s.current(&value).and_then(|c| s.choices[c].note.clone());
                    // The choice's line, why some choices are not here (once each), then what
                    // the row does.
                    let mut parts: Vec<&str> = note.as_deref().into_iter().collect();
                    for reason in s
                        .choices
                        .iter()
                        .filter(|c| !c.available)
                        .filter_map(|c| c.note.as_deref())
                    {
                        if !parts.contains(&reason) {
                            parts.push(reason);
                        }
                    }
                    parts.push(&s.help);
                    let text = parts.join(" ");
                    let status = if let Some(off) = &s.off {
                        Status::Off(off.clone())
                    } else if s.names().iter().any(|n| items::restart(settings, n)) {
                        Status::Later
                    } else {
                        Status::Live
                    };
                    (text, Some(status))
                }
                Some(Row::Action { help, off, .. }) => {
                    (help.clone(), off.as_ref().map(|o| Status::Off(o.clone())))
                }
                _ => (String::new(), None),
            },
            Focus::Button(0) => (
                "Applies the changed values and saves them for the next runs.".to_owned(),
                None,
            ),
            Focus::Button(1) => (
                "Puts this page's settings back to the engine's defaults, to keep with Apply."
                    .to_owned(),
                None,
            ),
            Focus::Button(_) => (
                "Back to the menu; changes not applied are asked about first.".to_owned(),
                None,
            ),
        }
    }
}

/// How the focused row applies, under its description.
enum Status {
    /// At once.
    Live,
    /// After a restart.
    Later,
    /// Not now: why.
    Off(String),
}

/// A rectangle's outline, `t` pixels thick.
fn frame(r: [f32; 4], t: f32, color: [f32; 4], out: &mut Vec<HudQuad>) {
    out.push(HudQuad::solid([r[0], r[1], r[2], r[1] + t], color));
    out.push(HudQuad::solid([r[0], r[3] - t, r[2], r[3]], color));
    out.push(HudQuad::solid([r[0], r[1] + t, r[0] + t, r[3] - t], color));
    out.push(HudQuad::solid([r[2] - t, r[1] + t, r[2], r[3] - t], color));
}

/// A row's value when none of its choices is in force (set by hand or by the console): a single
/// setting's value, or "Custom".
fn other(settings: &Settings, s: &Setting) -> String {
    match s.names().as_slice() {
        [name] => settings
            .info(name)
            .map_or_else(String::new, |v| v.value_text()),
        _ => "Custom".to_owned(),
    }
}

/// `text` in lines at most `width` wide as `measure` sets them, broken between words.
fn wrap(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && measure(&format!("{line} {word}")) > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The atlas cell of byte `c`: printable ASCII, anything else a `?`.
fn glyph(c: u8) -> usize {
    usize::from(if (32..127).contains(&c) { c } else { b'?' } - 32)
}

/// The menu's glyphs: ASCII rasterised in each [`Face`] at each of its sizes ([`Face::sizes`]),
/// white with the coverage as alpha, each in a cell padded by [`PAD`].
pub struct MenuFont {
    /// By [`Face`], smallest first; empty for a face whose file could not be read, which the
    /// mono face stands in for.
    faces: Vec<Vec<Atlas>>,
    metrics: Metrics,
}

struct Atlas {
    /// Kept alive while its texture is in use.
    _image: GraphImage,
    texture: u32,
    /// The em size it was rasterised at, pixels.
    em: f32,
    /// A glyph's cell without its padding, and the texture's size, pixels.
    cell: [f32; 2],
    size: [f32; 2],
    /// Each glyph's advance, the baseline's depth under a cell's top and the origin's distance
    /// from its left, pixels.
    advances: Vec<f32>,
    ascent: f32,
    left: f32,
}

impl MenuFont {
    /// Rasterises the mono face from `mono` (a TTF; the overlay's built-in pixel font when it is
    /// missing or unreadable) and the others from the fonts' folder above it (`assets/fonts`),
    /// and uploads the atlases.
    pub fn new(device: &Arc<Device>, mono: Option<&Path>) -> Result<Self> {
        let fonts = mono.and_then(|p| p.parent()?.parent());
        let mut faces = Vec::new();
        for face in Face::ALL {
            let path = match face.file() {
                None => mono.map(Path::to_path_buf),
                Some(file) => fonts.map(|dir| dir.join(file)),
            };
            let bytes = path.and_then(|p| std::fs::read(p).ok());
            let mut atlases = Vec::new();
            for &em in face.sizes() {
                let made = match bytes.as_deref().and_then(|b| rasterize_ttf(b, em)) {
                    Some(atlas) => Some((atlas, em)),
                    // The built-in font's 12 × 16 cells hold glyphs about 12 pixels tall.
                    None if face == Face::Mono => Some((rasterize_builtin(), 12.0)),
                    None => None,
                };
                if let Some((atlas, em)) = made {
                    atlases.push(Atlas::upload(device, &atlas, em)?);
                }
            }
            faces.push(atlases);
        }
        let big = faces[0].last().expect("a mono atlas");
        let metrics = Metrics {
            advance: (big.cell[0] - big.left) / big.em,
            height: big.cell[1] / big.em,
        };
        // The mono face on the grid the layout counts in, its big atlas's cells, at every size.
        for atlas in &mut faces[0] {
            atlas.advances.fill(metrics.advance * atlas.em);
        }
        Ok(Self { faces, metrics })
    }

    /// The atlas for text in `face` (the mono face's when its file was missing) `px` in size:
    /// the smallest at or above it.
    fn atlas(&self, face: Face, px: f32) -> &Atlas {
        let atlases = match &self.faces[face as usize] {
            none if none.is_empty() => &self.faces[0],
            atlases => atlases,
        };
        atlases
            .iter()
            .find(|a| a.em >= px * 0.95)
            .unwrap_or_else(|| atlases.last().expect("an atlas"))
    }

    /// `text`'s quads set in `style`, its baseline's left end at `at`, in `color` (linear,
    /// straight alpha).
    pub(crate) fn line(
        &self,
        out: &mut Vec<HudQuad>,
        style: Style,
        at: [f32; 2],
        color: [f32; 4],
        text: &str,
    ) {
        let atlas = self.atlas(style.face, style.px);
        let scale = style.px / atlas.em;
        let pad = PAD as f32;
        let (w, h) = (atlas.cell[0] + 2.0 * pad, atlas.cell[1] + 2.0 * pad);
        let top = at[1] - (atlas.ascent + pad) * scale;
        let mut x = at[0];
        for c in text.bytes() {
            let g = glyph(c);
            if c != b' ' {
                let (u, v) = (
                    (g % ATLAS_COLUMNS) as f32 * w / atlas.size[0],
                    (g / ATLAS_COLUMNS) as f32 * h / atlas.size[1],
                );
                let x0 = x - (pad + atlas.left) * scale;
                out.push(HudQuad {
                    rect: [x0, top, x0 + w * scale, top + h * scale],
                    uv: [u, v, u + w / atlas.size[0], v + h / atlas.size[1]],
                    texture: Some(atlas.texture),
                    color,
                });
            }
            x += atlas.advances[g] * scale + style.spacing * style.px;
        }
    }

    /// `text` in the mono face, its first cell's top left at `at`, `px` pixels in size: the base
    /// look's, and the games' HUD text ([`crate::Context::hud_text`]).
    pub(crate) fn text(
        &self,
        out: &mut Vec<HudQuad>,
        at: [f32; 2],
        px: f32,
        color: [f32; 4],
        text: &str,
    ) {
        let atlas = self.atlas(Face::Mono, px);
        let baseline = at[1] + atlas.ascent * px / atlas.em;
        let style = Style::new(Face::Mono, px, 0.0);
        self.line(out, style, [at[0], baseline], color, text);
    }

    /// How wide [`MenuFont::text`] sets `text` at `px` pixels.
    pub(crate) fn text_width(&self, px: f32, text: &str) -> f32 {
        self.width(Style::new(Face::Mono, px, 0.0), text)
    }
}

impl Measure for MenuFont {
    fn metrics(&self) -> Metrics {
        self.metrics
    }

    fn width(&self, style: Style, text: &str) -> f32 {
        let atlas = self.atlas(style.face, style.px);
        let advances: f32 = text.bytes().map(|c| atlas.advances[glyph(c)]).sum();
        let gaps = (text.len() as f32 - 1.0).max(0.0);
        advances * style.px / atlas.em + style.spacing * style.px * gaps
    }
}

impl Atlas {
    /// `atlas`'s glyphs in rows of [`ATLAS_COLUMNS`] (a single row of the wider faces' cells
    /// would pass the 4096 pixels every GPU takes), uploaded.
    fn upload(device: &Arc<Device>, atlas: &FontAtlas, em: f32) -> Result<Self> {
        let (cw, ch) = (atlas.cell_w as usize, atlas.cell_h as usize);
        let (pw, ph) = (cw + 2 * PAD as usize, ch + 2 * PAD as usize);
        let (width, height) = (pw * ATLAS_COLUMNS, ph * GLYPHS.div_ceil(ATLAS_COLUMNS));
        // White everywhere, so the linear filter never darkens an edge; the coverage is alpha.
        let mut rgba = [255_u8, 255, 255, 0].repeat(width * height);
        let stride = cw * GLYPHS;
        for g in 0..GLYPHS {
            let ox = (g % ATLAS_COLUMNS) * pw + PAD as usize;
            let oy = (g / ATLAS_COLUMNS) * ph + PAD as usize;
            for y in 0..ch {
                for x in 0..cw {
                    let coverage = atlas.pixels[y * stride + g * cw + x];
                    rgba[((oy + y) * width + ox + x) * 4 + 3] = coverage;
                }
            }
        }
        let image = GraphImage::uploaded(
            device,
            ImageDesc {
                width: width as u32,
                height: height as u32,
                format: vk::Format::R8G8B8A8_UNORM,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "menu font",
            },
            &rgba,
        )?;
        Ok(Self {
            texture: image.sampled().0,
            _image: image,
            em,
            cell: [cw as f32, ch as f32],
            size: [width as f32, height as f32],
            advances: atlas.advances.clone(),
            ascent: atlas.ascent,
            left: atlas.left,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::settings::{Source, Spec, Value};

    const M: Metrics = Metrics {
        advance: 0.6,
        height: 1.3,
    };
    const SIZE: [f32; 2] = [1920.0, 1080.0];

    fn settings() -> Settings {
        let mut s = Settings::new();
        s.add(Spec::float("snd.volume", 1.0, 0.0..=1.0, "volume"));
        s.add(Spec::bool("app.mouse_look", false, "look"));
        s.add(Spec::choice("app.menu_theme", &Theme::NAMES, 0, "theme"));
        s.add(Spec::int("r.display_info", 0, 0..=2, "info"));
        s.add(Spec::choice(
            "r.preset",
            crate::presets::PRESETS,
            0,
            "preset",
        ));
        s.add(Spec::choice(
            "r.aa",
            &["off", "dlaa", "dlss", "fsr"],
            1,
            "aa",
        ));
        s.add(Spec::bool("r.ao", true, "ao"));
        s.add(Spec::bool("r.reflections.rays", true, "rays"));
        s.add(Spec::float("r.lod.error", 1.0, 0.125..=16.0, "lod"));
        s
    }

    fn menu_on(tab: Tab) -> Menu {
        let mut menu = Menu::default();
        menu.set_caps(Caps {
            dlss: true,
            window: [1920, 1080],
            ..Caps::default()
        });
        menu.open_settings(tab);
        menu
    }

    /// The tab's rows' labels, headers left out, and their values as drawn.
    fn values(menu: &Menu, s: &Settings) -> Vec<(String, String)> {
        let value = |n: &str| menu.draft.value(s, n);
        menu.rows(s)
            .iter()
            .filter_map(|r| match r {
                Row::Setting(row) => Some((
                    row.label.to_owned(),
                    row.current(&value)
                        .map_or_else(|| other(s, row), |i| row.choices[i].label.clone()),
                )),
                _ => None,
            })
            .collect()
    }

    fn value_of(menu: &Menu, s: &Settings, label: &str) -> String {
        values(menu, s)
            .into_iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v)
            .unwrap_or_default()
    }

    /// The focus moved down to the row labelled `label`.
    fn focus_on(menu: &mut Menu, s: &mut Settings, label: &str) {
        menu.focus = Focus::Row(0);
        menu.settle_focus(s);
        for _ in 0..32 {
            if let Focus::Row(i) = menu.focus
                && let Some(Row::Setting(r)) = menu.rows(s).get(i)
                && r.label == label
            {
                return;
            }
            menu.key(KeyCode::ArrowDown, s);
        }
        panic!("no row {label}");
    }

    #[test]
    fn esc_opens_its_first_page_and_resumes_from_it() {
        let mut s = settings();
        let mut menu = Menu::default();
        menu.open();
        assert!(menu.is_open());
        assert_eq!(menu.key(KeyCode::ArrowDown, &mut s), None);
        assert_eq!(menu.key(KeyCode::ArrowDown, &mut s), None);
        assert_eq!(menu.key(KeyCode::Enter, &mut s), Some(Action::Quit));
        assert_eq!(menu.key(KeyCode::Escape, &mut s), Some(Action::Resume));
        assert!(!menu.is_open());
        // Opened again, on Resume.
        menu.open();
        assert_eq!(menu.key(KeyCode::Space, &mut s), Some(Action::Resume));
        // Settings opens on the graphics tab, its first row focused.
        menu.open();
        menu.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(menu.key(KeyCode::Enter, &mut s), None);
        assert_eq!(menu.page, Page::Settings);
        assert_eq!(menu.tab, Tab::Graphics);
        assert!(matches!(menu.focus, Focus::Row(i) if menu.rows(&s)[i].focusable()));
    }

    #[test]
    fn changes_wait_for_apply_and_are_the_users() {
        let mut s = settings();
        let mut menu = menu_on(Tab::Graphics);
        focus_on(&mut menu, &mut s, "Ambient occlusion");
        menu.key(KeyCode::ArrowLeft, &mut s);
        // Drawn changed, not applied.
        assert_eq!(value_of(&menu, &s, "Ambient occlusion"), "Off");
        assert_eq!(s.info("r.ao").map(|v| v.value), Some(Value::Bool(true)));
        // Left at the first choice stays there.
        menu.key(KeyCode::ArrowLeft, &mut s);
        assert_eq!(value_of(&menu, &s, "Ambient occlusion"), "Off");
        // Down past the last row reaches Apply.
        for _ in 0..16 {
            menu.key(KeyCode::ArrowDown, &mut s);
        }
        assert_eq!(menu.focus, Focus::Button(0));
        menu.key(KeyCode::Enter, &mut s);
        assert_eq!(s.info("r.ao").map(|v| v.value), Some(Value::Bool(false)));
        assert_eq!(s.info("r.ao").map(|v| v.source), Some(Source::User));
        assert!(s.take_user_changed());
        assert_eq!(s.user_lines(), ["r.ao 0"]);
        assert_eq!(menu.notice.as_deref(), Some("Applied and saved."));
        // Nothing left to apply: Esc goes straight back.
        assert_eq!(menu.key(KeyCode::Escape, &mut s), None);
        assert_eq!(menu.page, Page::Main);
    }

    #[test]
    fn leaving_with_changes_asks_first() {
        let mut s = settings();
        let mut menu = menu_on(Tab::Graphics);
        focus_on(&mut menu, &mut s, "Ray-traced reflections");
        menu.key(KeyCode::Enter, &mut s);
        menu.key(KeyCode::Escape, &mut s);
        assert_eq!(menu.dialog, Some(0));
        // Cancel stays, the change kept.
        menu.key(KeyCode::Escape, &mut s);
        assert_eq!(menu.dialog, None);
        assert_eq!(menu.page, Page::Settings);
        assert_eq!(value_of(&menu, &s, "Ray-traced reflections"), "Off");
        // Discard leaves and forgets it.
        menu.key(KeyCode::Escape, &mut s);
        menu.key(KeyCode::ArrowRight, &mut s);
        menu.key(KeyCode::Enter, &mut s);
        assert_eq!(menu.page, Page::Main);
        assert_eq!(
            s.info("r.reflections.rays").map(|v| v.value),
            Some(Value::Bool(true))
        );
        assert!(s.user_lines().is_empty());
        // Apply in the dialog applies, then leaves.
        menu.open_settings(Tab::Graphics);
        focus_on(&mut menu, &mut s, "Ray-traced reflections");
        menu.key(KeyCode::Enter, &mut s);
        menu.key(KeyCode::Escape, &mut s);
        menu.key(KeyCode::Enter, &mut s);
        assert_eq!(menu.page, Page::Main);
        assert_eq!(s.user_lines(), ["r.reflections.rays 0"]);
    }

    #[test]
    fn a_preset_sets_its_rows_and_a_row_changed_makes_it_custom() {
        let mut s = settings();
        let mut menu = menu_on(Tab::Graphics);
        focus_on(&mut menu, &mut s, "Quality preset");
        menu.key(KeyCode::ArrowRight, &mut s);
        assert_eq!(value_of(&menu, &s, "Quality preset"), "Low");
        assert_eq!(value_of(&menu, &s, "Ambient occlusion"), "Off");
        assert_eq!(value_of(&menu, &s, "Level of detail"), "Low (3 px)");
        focus_on(&mut menu, &mut s, "Ambient occlusion");
        menu.key(KeyCode::ArrowRight, &mut s);
        assert_eq!(value_of(&menu, &s, "Quality preset"), "Custom");
        // Defaults: the page's rows back, preset included.
        menu.focus = Focus::Button(1);
        menu.key(KeyCode::Enter, &mut s);
        assert_eq!(value_of(&menu, &s, "Quality preset"), "Custom");
        assert_eq!(value_of(&menu, &s, "Level of detail"), "High (1 px)");
        assert!(!menu.draft.is_dirty(&s));
    }

    #[test]
    fn tabs_turn_and_keep_the_draft() {
        let mut s = settings();
        let mut menu = menu_on(Tab::Graphics);
        focus_on(&mut menu, &mut s, "Ambient occlusion");
        menu.key(KeyCode::Enter, &mut s);
        menu.key(KeyCode::Tab, &mut s);
        assert_eq!(menu.tab, Tab::Display);
        menu.key(KeyCode::Tab, &mut s);
        assert_eq!(menu.tab, Tab::Game);
        assert_eq!(
            values(&menu, &s),
            [
                ("Volume".to_owned(), "100 %".to_owned()),
                ("Mouse look".to_owned(), "Off".to_owned()),
                ("Frame info".to_owned(), "Off".to_owned()),
            ]
        );
        focus_on(&mut menu, &mut s, "Volume");
        menu.key(KeyCode::ArrowLeft, &mut s);
        menu.key(KeyCode::ArrowLeft, &mut s);
        assert_eq!(value_of(&menu, &s, "Volume"), "80 %");
        menu.key(KeyCode::KeyQ, &mut s);
        menu.key(KeyCode::KeyQ, &mut s);
        assert_eq!(menu.tab, Tab::Graphics);
        assert_eq!(value_of(&menu, &s, "Ambient occlusion"), "Off");
        menu.focus = Focus::Button(0);
        menu.key(KeyCode::Space, &mut s);
        assert_eq!(s.user_lines(), ["r.ao 0", "snd.volume 0.8"]);
    }

    #[test]
    fn the_mouse_chooses_by_hovering_and_takes_by_a_click() {
        let mut s = settings();
        let mut menu = Menu::default();
        menu.open();
        let rows = menu.layout(SIZE, &M, &s).rows;
        let middle = |r: [f32; 4]| [0.5 * (r[0] + r[2]), 0.5 * (r[1] + r[3])];
        menu.hover(middle(rows[2]), SIZE, &M, &s);
        assert_eq!(menu.selected, 2);
        // Off every entry, the choice stays.
        menu.hover([1.0, 1.0], SIZE, &M, &s);
        assert_eq!(menu.selected, 2);
        assert_eq!(menu.click([1.0, 1.0], SIZE, &M, &mut s, false), None);
        assert_eq!(
            menu.click(middle(rows[0]), SIZE, &M, &mut s, false),
            Some(Action::Resume)
        );
        // On the settings screen: a tab by a click, a row stepped forward and back.
        menu.open_settings(Tab::Graphics);
        let l = menu.layout(SIZE, &M, &s);
        menu.click(middle(l.tabs[2]), SIZE, &M, &mut s, false);
        assert_eq!(menu.tab, Tab::Game);
        let l = menu.layout(SIZE, &M, &s);
        let volume = 1; // under the SOUND header
        menu.click(middle(l.rows[volume]), SIZE, &M, &mut s, true);
        assert_eq!(value_of(&menu, &s, "Volume"), "90 %");
        menu.click(middle(l.rows[volume]), SIZE, &M, &mut s, false);
        menu.click(middle(l.rows[volume]), SIZE, &M, &mut s, false);
        assert_eq!(value_of(&menu, &s, "Volume"), "0 %");
        // A section's title takes no click, whatever has the focus (it stays on Volume).
        menu.click(middle(l.rows[0]), SIZE, &M, &mut s, false);
        assert_eq!(value_of(&menu, &s, "Volume"), "0 %");
        menu.focus = Focus::Button(1);
        menu.click(middle(l.rows[0]), SIZE, &M, &mut s, false);
        assert_eq!(value_of(&menu, &s, "Volume"), "0 %");
        // Apply's button.
        menu.click(middle(l.buttons[0]), SIZE, &M, &mut s, false);
        assert_eq!(s.user_lines(), ["snd.volume 0"]);
        // A value `--set` gave, chosen as it is, is kept for Apply to make it the user's.
        // Chosen as it is, the user's own value leaves the draft.
        s.set_text("r.display_info", "2", Source::CommandLine)
            .unwrap();
        menu.draft.set(&s, "r.display_info", "2");
        menu.draft.set(&s, "snd.volume", "0");
        assert!(menu.draft.changes(&s, "r.display_info"));
        assert!(!menu.draft.changes(&s, "snd.volume"));
    }

    #[test]
    fn the_layout_fits_the_window_from_900p_to_4k_in_both_looks() {
        let mut s = settings();
        for theme in Theme::NAMES {
            s.set_text("app.menu_theme", theme, Source::Console)
                .unwrap();
            for size in [
                [1600.0, 900.0],
                SIZE,
                [2560.0, 1440.0],
                [3840.0, 2160.0],
                [1280.0, 720.0],
            ] {
                for tab in Tab::ALL {
                    let mut menu = menu_on(tab);
                    menu.dialog = Some(0);
                    let l = menu.layout(size, &M, &s);
                    // The rows end above the description, the buttons above the hint, all
                    // inside; the tabs side by side.
                    let last = l.rows.last().copied().unwrap_or_default();
                    assert!(
                        last[3] <= l.description[1] + 1e-3,
                        "{theme} {size:?} {tab:?}"
                    );
                    assert!(l.description[3] <= l.buttons[0][1] + 1e-3);
                    assert!(
                        l.buttons
                            .iter()
                            .all(|b| b[3] <= l.hint.0 && b[0] >= l.inner[0])
                    );
                    assert!(l.hint.0 + l.hint.1 <= size[1]);
                    assert!(l.column[0] >= 0.0 && l.column[2] <= size[0]);
                    for w in l.rows.windows(2) {
                        assert!(w[0][3] <= w[1][1] + 1e-3);
                    }
                    for w in l.tabs.windows(2) {
                        assert!(w[0][2] < w[1][0]);
                    }
                    assert!(l.tabs.iter().all(|t| t[2] <= l.inner[1]));
                    // The dialog's buttons inside its panel.
                    let (panel, buttons) = l.dialog.expect("the dialog");
                    assert!(buttons[0][0] >= panel[0] && buttons[2][2] <= panel[2]);
                    // Readable: about 23 pixels at 1080p, twice at 4K.
                    assert!(l.px >= 13.0, "{size:?}: {}", l.px);
                }
            }
            let l = menu_on(Tab::Graphics).layout(SIZE, &M, &s);
            assert!((l.px - 1080.0 / 46.0).abs() < 0.01, "{}", l.px);
            let l4 = menu_on(Tab::Graphics).layout([3840.0, 2160.0], &M, &s);
            assert!((l4.px - 2.0 * l.px).abs() < 0.02);
        }
    }

    #[test]
    fn the_theme_is_carbon_unless_base_is_chosen() {
        let mut s = Settings::new();
        assert_eq!(Theme::of(&s), Theme::Carbon);
        s.add(Spec::choice("app.menu_theme", &Theme::NAMES, 0, "theme"));
        assert_eq!(Theme::of(&s), Theme::Carbon);
        s.set_text("app.menu_theme", "base", Source::Console)
            .unwrap();
        assert_eq!(Theme::of(&s), Theme::Base);
    }

    #[test]
    fn text_wraps_between_words() {
        let chars = |s: &str| s.len() as f32;
        assert_eq!(
            wrap("one two three four", 9.0, chars),
            ["one two", "three", "four"]
        );
        assert!(wrap("", 9.0, chars).is_empty());
    }
}

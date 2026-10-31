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

pub mod items;

use std::path::Path;
use std::sync::Arc;

use forge_core::settings::Settings;
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
/// The column's width in characters of the body text.
const COLUMNS: f32 = 66.0;

/// The em sizes the glyphs are rasterised at, pixels: text takes the smallest at or above its
/// size, so it is drawn at 0.5 to 1.2 of an atlas.
const ATLAS_PX: [f32; 2] = [26.0, 56.0];
/// Empty pixels round each glyph in an atlas, so the linear filter never reads a neighbour.
const PAD: u32 = 2;

/// The colours, linear with straight alpha.
/// Over the whole window where the glass cannot be drawn.
const DIM: [f32; 4] = [0.004, 0.005, 0.007, 0.86];
const COLUMN: [f32; 4] = [1.0, 1.0, 1.0, 0.025];
const EDGE: [f32; 4] = [1.0, 1.0, 1.0, 0.10];
const RULE: [f32; 4] = [1.0, 1.0, 1.0, 0.13];
const BAR: [f32; 4] = [1.0, 1.0, 1.0, 0.08];
const ACCENT: [f32; 4] = [0.95, 0.62, 0.22, 1.0];
const TEXT: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
const CHOSEN: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const TITLE: [f32; 4] = [0.95, 0.62, 0.22, 1.0];
const HINT: [f32; 4] = [0.40, 0.40, 0.40, 1.0];
const OFF: [f32; 4] = [0.20, 0.20, 0.20, 1.0];
const LATER: [f32; 4] = [0.80, 0.64, 0.36, 1.0];
const LIVE: [f32; 4] = [0.42, 0.66, 0.46, 1.0];
const DIALOG_PANEL: [f32; 4] = [0.018, 0.020, 0.026, 0.97];

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

/// A glyph cell's size for each pixel of the text's size: its advance and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    advance: f32,
    height: f32,
}

impl Metrics {
    /// How wide `text` is at `px` pixels, its cells one advance each.
    pub fn width(self, px: f32, text: &str) -> f32 {
        self.advance * px * text.len() as f32
    }
}

/// Where everything goes, the window's pixels.
#[derive(Debug, Default)]
struct Layout {
    /// The body text's size.
    px: f32,
    /// The column, top to bottom of the window.
    column: [f32; 4],
    /// Its inside, left and right, past the margins.
    inner: [f32; 2],
    /// The title's top left and size.
    title: ([f32; 2], f32),
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
    /// The hint's top and size.
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
    pub fn hover(&mut self, at: [f32; 2], size: [f32; 2], m: Metrics, settings: &Settings) {
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
        m: Metrics,
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
    fn under(&self, at: [f32; 2], size: [f32; 2], m: Metrics, settings: &Settings) -> Option<Hit> {
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

    /// The column down the window's middle: the title, the tabs, the rows, the description, the
    /// buttons and the hint; the text sized to the window and to fit it.
    fn layout(&self, size: [f32; 2], m: Metrics, settings: &Settings) -> Layout {
        let [w, h] = size;
        let rows = match self.page {
            Page::Main => Vec::new(),
            Page::Settings => self.rows(settings),
        };
        // About 23 pixels at 1080p, 47 at 4K; smaller when the rows or the width need it.
        let lines = match self.page {
            Page::Main => 20.0,
            Page::Settings => 17.5 + 1.55 * rows.len() as f32,
        };
        let px = (h / 46.0)
            .clamp(13.0, 64.0)
            .min(h / lines)
            .min(0.94 * w / (COLUMNS * m.advance));
        let width = COLUMNS * m.advance * px;
        let left = 0.5 * (w - width);
        let margin = 2.0 * m.advance * px;
        let inner = [left + margin, left + width - margin];
        let top = 1.3 * px;
        let hint_px = (0.62 * px).max(11.0);
        let hint_top = h - 1.4 * px - hint_px;
        let mut l = Layout {
            px,
            column: [left, 0.0, left + width, h],
            inner,
            hint: (hint_top, hint_px),
            ..Layout::default()
        };
        match self.page {
            Page::Main => {
                let entry = 1.25 * px;
                let line = 2.0 * px;
                let block = line * MAIN.len() as f32;
                let first = 0.5 * (h - block) + 0.6 * px;
                l.title = ([inner[0], first - 3.4 * px], 2.0 * px);
                l.rows = (0..MAIN.len())
                    .map(|k| {
                        let y = first + line * k as f32;
                        [left, y, left + width, y + line]
                    })
                    .collect();
                l.px = entry;
            }
            Page::Settings => {
                l.title = ([inner[0], top], 1.45 * px);
                let tabs_top = top + 2.5 * px;
                let tab_h = 1.9 * px;
                let mut x = inner[0];
                for tab in Tab::ALL {
                    let tw = m.width(0.95 * px, tab.label());
                    l.tabs.push([x, tabs_top, x + tw, tabs_top + tab_h]);
                    x += tw + 2.5 * m.advance * px;
                }
                l.rule = tabs_top + tab_h;
                let line = 1.55 * px;
                let first = l.rule + 0.7 * px;
                l.rows = (0..rows.len())
                    .map(|k| {
                        let y = first + line * k as f32;
                        [left, y, left + width, y + line]
                    })
                    .collect();
                let button_h = 1.9 * px;
                let buttons_bottom = hint_top - 0.7 * px;
                let mut x = inner[1];
                let mut buttons = Vec::new();
                for label in BUTTONS.iter().rev() {
                    let bw = m.width(px, label) + 3.0 * m.advance * px;
                    buttons.push([x - bw, buttons_bottom - button_h, x, buttons_bottom]);
                    x -= bw + 1.2 * m.advance * px;
                }
                buttons.reverse();
                l.buttons = buttons;
                let description_bottom = buttons_bottom - button_h - 0.8 * px;
                l.description = [
                    inner[0],
                    description_bottom - 4.9 * px,
                    inner[1],
                    description_bottom,
                ];
            }
        }
        if self.dialog.is_some() {
            let dw = (46.0 * m.advance * px).min(0.96 * w);
            let dh = 7.6 * px;
            let (dx, dy) = (0.5 * (w - dw), 0.5 * (h - dh));
            let mut x = dx + dw - 1.6 * m.advance * px;
            let bottom = dy + dh - 1.1 * px;
            let mut buttons = Vec::new();
            for label in DIALOG.iter().rev() {
                let bw = m.width(px, label) + 3.0 * m.advance * px;
                buttons.push([x - bw, bottom - 1.9 * px, x, bottom]);
                x -= bw + 1.2 * m.advance * px;
            }
            buttons.reverse();
            l.dialog = Some(([dx, dy, dx + dw, dy + dh], buttons));
        }
        l
    }

    /// Its quads for a window of `size` pixels, over everything else: `paused` when the game
    /// holds behind it, `glass` when the shell drew the frosted glass under it (else the game is
    /// dimmed here).
    pub fn draw(
        &self,
        font: &MenuFont,
        size: [f32; 2],
        settings: &Settings,
        paused: bool,
        glass: bool,
        out: &mut Vec<HudQuad>,
    ) {
        let m = font.metrics();
        let l = self.layout(size, m, settings);
        if !glass {
            out.push(HudQuad::solid([0.0, 0.0, size[0], size[1]], DIM));
        }
        let hair = (0.05 * l.px).max(1.0);
        out.push(HudQuad::solid(l.column, COLUMN));
        let [left, top, right, bottom] = l.column;
        out.push(HudQuad::solid([left, top, left + hair, bottom], EDGE));
        out.push(HudQuad::solid([right - hair, top, right, bottom], EDGE));
        match self.page {
            Page::Main => self.draw_main(font, &l, paused, out),
            Page::Settings => self.draw_settings(font, &l, settings, hair, out),
        }
        if let Some((panel, buttons)) = &l.dialog {
            self.draw_dialog(font, &l, *panel, buttons, hair, out);
        }
    }

    fn draw_main(&self, font: &MenuFont, l: &Layout, paused: bool, out: &mut Vec<HudQuad>) {
        let m = font.metrics();
        let title = if paused { "PAUSED" } else { "MENU" };
        font.text(out, l.title.0, l.title.1, TITLE, title);
        for (k, rect) in l.rows.iter().enumerate() {
            let chosen = k == self.selected;
            if chosen {
                out.push(HudQuad::solid(*rect, BAR));
                out.push(HudQuad::solid(
                    [rect[0], rect[1], rect[0] + 0.14 * l.px, rect[3]],
                    ACCENT,
                ));
            }
            let y = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * l.px);
            let color = if chosen { CHOSEN } else { TEXT };
            font.text(out, [l.inner[0], y], l.px, color, MAIN[k]);
        }
        let hint = if paused {
            "Esc: back to the game"
        } else {
            "The game goes on behind the menu"
        };
        font.text(out, [l.inner[0], l.hint.0], l.hint.1, HINT, hint);
    }

    fn draw_settings(
        &self,
        font: &MenuFont,
        l: &Layout,
        settings: &Settings,
        hair: f32,
        out: &mut Vec<HudQuad>,
    ) {
        let m = font.metrics();
        let px = l.px;
        font.text(out, l.title.0, l.title.1, TITLE, "SETTINGS");
        // The tabs, the chosen one underlined.
        for (k, rect) in l.tabs.iter().enumerate() {
            let on = Tab::ALL[k] == self.tab;
            let y = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * 0.95 * px);
            font.text(
                out,
                [rect[0], y],
                0.95 * px,
                if on { CHOSEN } else { TEXT },
                Tab::ALL[k].label(),
            );
            if on {
                out.push(HudQuad::solid(
                    [rect[0], rect[3] - 0.13 * px, rect[2], rect[3]],
                    ACCENT,
                ));
            }
        }
        out.push(HudQuad::solid(
            [l.inner[0], l.rule, l.inner[1], l.rule + hair],
            RULE,
        ));
        let rows = self.rows(settings);
        let focus = self.focused(&rows);
        let value = |n: &str| self.draft.value(settings, n);
        for (k, (row, rect)) in rows.iter().zip(&l.rows).enumerate() {
            let focused = focus == Focus::Row(k);
            let y = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * px);
            match row {
                Row::Header(title) => {
                    let hp = 0.7 * px;
                    let hy = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * hp) + 0.15 * px;
                    font.text(out, [l.inner[0], hy], hp, HINT, title);
                    let x = l.inner[0] + m.width(hp, title) + 1.2 * m.advance * px;
                    let mid = hy + 0.55 * m.height * hp;
                    out.push(HudQuad::solid([x, mid, l.inner[1], mid + hair], RULE));
                }
                Row::Setting(s) => {
                    if focused {
                        self.bar(*rect, px, out);
                    }
                    let off = s.off.is_some();
                    let color = match (off, focused) {
                        (true, _) => OFF,
                        (false, true) => CHOSEN,
                        (false, false) => TEXT,
                    };
                    font.text(out, [l.inner[0], y], px, color, s.label);
                    if s.names().iter().any(|n| items::restart(settings, n)) {
                        let x = l.inner[0] + m.width(px, s.label) + 1.5 * m.advance * px;
                        let tp = 0.7 * px;
                        font.text(
                            out,
                            [x, y + 0.5 * m.height * (px - tp)],
                            tp,
                            LATER,
                            "restart",
                        );
                    }
                    let changed = s.names().iter().any(|n| self.draft.changes(settings, n));
                    let shown = match (&s.shown_off, s.off.is_some()) {
                        (Some(text), true) => text.clone(),
                        _ => match s.current(&value) {
                            Some(i) => s.choices[i].label.clone(),
                            None => other(settings, s),
                        },
                    };
                    let shown = match (focused && !off, changed) {
                        (true, true) => format!("< {shown} > *"),
                        (true, false) => format!("< {shown} >  "),
                        (false, true) => format!("{shown} *"),
                        (false, false) => format!("{shown}  "),
                    };
                    let vcolor = if changed && !off { ACCENT } else { color };
                    let x = l.inner[1] - m.width(px, &shown);
                    font.text(out, [x, y], px, vcolor, &shown);
                }
                Row::Action { label, off, .. } => {
                    if focused {
                        self.bar(*rect, px, out);
                    }
                    let color = match (off.is_some(), focused) {
                        (true, _) => OFF,
                        (false, true) => CHOSEN,
                        (false, false) => TEXT,
                    };
                    font.text(out, [l.inner[0], y], px, color, label);
                    let x = l.inner[1] - m.width(px, "Open >  ");
                    font.text(out, [x, y], px, color, "Open >  ");
                }
            }
        }
        // What the focused row does, and whether it applies at once.
        let d = l.description;
        out.push(HudQuad::solid([d[0], d[1], d[2], d[1] + hair], RULE));
        let dp = 0.8 * px;
        let chars = ((d[2] - d[0]) / (m.advance * dp)).floor().max(10.0) as usize;
        let mut y = d[1] + 0.6 * px;
        let line = 1.35 * dp;
        let (text, status) = self.describe(settings, &rows, focus);
        for text_line in wrap(&text, chars).into_iter().take(3) {
            font.text(out, [d[0], y], dp, TEXT, &text_line);
            y += line;
        }
        if let Some((status, color)) = status {
            font.text(out, [d[0], y], dp, color, &status);
        }
        // The buttons; Apply bright only with something to apply.
        let dirty = self.draft.is_dirty(settings);
        for (k, rect) in l.buttons.iter().enumerate() {
            let focused = focus == Focus::Button(k);
            let live = k != 0 || dirty;
            let border = match (focused, live) {
                (true, true) => ACCENT,
                (_, false) => RULE,
                (false, true) => EDGE,
            };
            if focused {
                out.push(HudQuad::solid(*rect, BAR));
            }
            frame(*rect, hair.max(0.06 * px), border, out);
            let color = match (live, focused, k) {
                (false, _, _) => OFF,
                (true, true, _) => CHOSEN,
                (true, false, 0) => ACCENT,
                (true, false, _) => TEXT,
            };
            let tx = 0.5 * (rect[0] + rect[2] - m.width(px, BUTTONS[k]));
            let ty = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * px);
            font.text(out, [tx, ty], px, color, BUTTONS[k]);
        }
        let (hint, color) = match &self.notice {
            Some(notice) => (notice.as_str(), LIVE),
            None => (
                "Up/Down choose  Left/Right change  Tab next page  Enter take  Esc back",
                HINT,
            ),
        };
        font.text(out, [l.inner[0], l.hint.0], l.hint.1, color, hint);
    }

    /// The description of the focused row, and its status line: whether it applies at once, after
    /// a restart, or why it is greyed.
    fn describe(
        &self,
        settings: &Settings,
        rows: &[Row],
        focus: Focus,
    ) -> (String, Option<(String, [f32; 4])>) {
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
                        (off.clone(), LATER)
                    } else if s.names().iter().any(|n| items::restart(settings, n)) {
                        ("Applies after you restart.".to_owned(), LATER)
                    } else {
                        ("Applies at once.".to_owned(), LIVE)
                    };
                    (text, Some(status))
                }
                Some(Row::Action { help, off, .. }) => {
                    (help.clone(), off.as_ref().map(|o| (o.clone(), LATER)))
                }
                _ => (String::new(), None),
            },
            Focus::Button(0) => (
                "Applies the changes marked * and saves them for the next runs.".to_owned(),
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

    fn bar(&self, rect: [f32; 4], px: f32, out: &mut Vec<HudQuad>) {
        out.push(HudQuad::solid(rect, BAR));
        out.push(HudQuad::solid(
            [rect[0], rect[1], rect[0] + 0.14 * px, rect[3]],
            ACCENT,
        ));
    }

    fn draw_dialog(
        &self,
        font: &MenuFont,
        l: &Layout,
        panel: [f32; 4],
        buttons: &[[f32; 4]],
        hair: f32,
        out: &mut Vec<HudQuad>,
    ) {
        let m = font.metrics();
        let px = l.px;
        out.push(HudQuad::solid(panel, DIALOG_PANEL));
        frame(panel, hair, EDGE, out);
        let x = panel[0] + 1.6 * m.advance * px;
        font.text(
            out,
            [x, panel[1] + 1.0 * px],
            1.1 * px,
            CHOSEN,
            "Apply the changes?",
        );
        font.text(
            out,
            [x, panel[1] + 2.8 * px],
            0.8 * px,
            TEXT,
            "Some settings changed and are not applied yet.",
        );
        for (k, rect) in buttons.iter().enumerate() {
            let focused = self.dialog == Some(k);
            if focused {
                out.push(HudQuad::solid(*rect, BAR));
            }
            frame(
                *rect,
                hair.max(0.06 * px),
                if focused { ACCENT } else { EDGE },
                out,
            );
            let tx = 0.5 * (rect[0] + rect[2] - m.width(px, DIALOG[k]));
            let ty = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * px);
            font.text(
                out,
                [tx, ty],
                px,
                if focused { CHOSEN } else { TEXT },
                DIALOG[k],
            );
        }
    }
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

/// `text` in lines of at most `width` characters, broken between words.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
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

/// The menu's glyphs: ASCII rasterised at each of [`ATLAS_PX`] from the overlay's font, white
/// with the coverage as alpha, each in a cell padded by [`PAD`].
pub struct MenuFont {
    atlases: Vec<Atlas>,
    metrics: Metrics,
}

struct Atlas {
    /// Kept alive while its texture is in use.
    _image: GraphImage,
    texture: u32,
    /// The em size it was rasterised at, pixels.
    em: f32,
    /// A glyph's cell without its padding, and the texture's width, pixels.
    cell: [f32; 2],
    width: f32,
}

impl MenuFont {
    /// Rasterises `font` (a TTF), or the overlay's built-in pixel font when it is missing or
    /// unreadable, and uploads the atlases.
    pub fn new(device: &Arc<Device>, font: Option<&Path>) -> Result<Self> {
        let bytes = font.and_then(|p| std::fs::read(p).ok());
        let atlases = ATLAS_PX
            .iter()
            .map(|&em| {
                let (atlas, em) = match bytes.as_deref().and_then(|b| rasterize_ttf(b, em)) {
                    Some(atlas) => (atlas, em),
                    // The built-in font's 12 × 16 cells hold glyphs about 12 pixels tall.
                    None => (rasterize_builtin(), 12.0),
                };
                Atlas::upload(device, &atlas, em)
            })
            .collect::<Result<Vec<_>>>()?;
        let big = atlases.last().expect("an atlas");
        let metrics = Metrics {
            advance: big.cell[0] / big.em,
            height: big.cell[1] / big.em,
        };
        Ok(Self { atlases, metrics })
    }

    /// The cells' size per pixel of text size, for the layout.
    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// `text`'s quads, its cells' top left corner at `at`, `px` pixels in size.
    pub(crate) fn text(
        &self,
        out: &mut Vec<HudQuad>,
        at: [f32; 2],
        px: f32,
        color: [f32; 4],
        text: &str,
    ) {
        let atlas = self
            .atlases
            .iter()
            .find(|a| a.em >= px * 0.95)
            .unwrap_or_else(|| self.atlases.last().expect("an atlas"));
        let scale = px / atlas.em;
        let advance = self.metrics.advance * px;
        let pad = PAD as f32;
        let (w, h) = (atlas.cell[0] + 2.0 * pad, atlas.cell[1] + 2.0 * pad);
        for (k, c) in text.bytes().enumerate() {
            if c == b' ' {
                continue;
            }
            let glyph = if (32..127).contains(&c) {
                c - 32
            } else {
                b'?' - 32
            };
            let x = at[0] + advance * k as f32 - pad * scale;
            let y = at[1] - pad * scale;
            let u = f32::from(glyph) * w / atlas.width;
            out.push(HudQuad {
                rect: [x, y, x + w * scale, y + h * scale],
                uv: [u, 0.0, u + w / atlas.width, 1.0],
                texture: Some(atlas.texture),
                color,
            });
        }
    }
}

impl Atlas {
    fn upload(device: &Arc<Device>, atlas: &FontAtlas, em: f32) -> Result<Self> {
        let (cw, ch) = (atlas.cell_w, atlas.cell_h);
        let (pw, ph) = (cw + 2 * PAD, ch + 2 * PAD);
        let width = pw * GLYPHS as u32;
        // White everywhere, so the linear filter never darkens an edge; the coverage is alpha.
        let mut rgba = [255_u8, 255, 255, 0].repeat((width * ph) as usize);
        let stride = cw as usize * GLYPHS;
        for g in 0..GLYPHS {
            for y in 0..ch as usize {
                for x in 0..cw as usize {
                    let coverage = atlas.pixels[y * stride + g * cw as usize + x];
                    let (dx, dy) = (g * pw as usize + PAD as usize + x, PAD as usize + y);
                    rgba[(dy * width as usize + dx) * 4 + 3] = coverage;
                }
            }
        }
        let image = GraphImage::uploaded(
            device,
            ImageDesc {
                width,
                height: ph,
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
            width: width as f32,
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
        let rows = menu.layout(SIZE, M, &s).rows;
        let middle = |r: [f32; 4]| [0.5 * (r[0] + r[2]), 0.5 * (r[1] + r[3])];
        menu.hover(middle(rows[2]), SIZE, M, &s);
        assert_eq!(menu.selected, 2);
        // Off every entry, the choice stays.
        menu.hover([1.0, 1.0], SIZE, M, &s);
        assert_eq!(menu.selected, 2);
        assert_eq!(menu.click([1.0, 1.0], SIZE, M, &mut s, false), None);
        assert_eq!(
            menu.click(middle(rows[0]), SIZE, M, &mut s, false),
            Some(Action::Resume)
        );
        // On the settings screen: a tab by a click, a row stepped forward and back.
        menu.open_settings(Tab::Graphics);
        let l = menu.layout(SIZE, M, &s);
        menu.click(middle(l.tabs[2]), SIZE, M, &mut s, false);
        assert_eq!(menu.tab, Tab::Game);
        let l = menu.layout(SIZE, M, &s);
        let volume = 1; // under the SOUND header
        menu.click(middle(l.rows[volume]), SIZE, M, &mut s, true);
        assert_eq!(value_of(&menu, &s, "Volume"), "90 %");
        menu.click(middle(l.rows[volume]), SIZE, M, &mut s, false);
        menu.click(middle(l.rows[volume]), SIZE, M, &mut s, false);
        assert_eq!(value_of(&menu, &s, "Volume"), "0 %");
        // A section's title takes no click, whatever has the focus (it stays on Volume).
        menu.click(middle(l.rows[0]), SIZE, M, &mut s, false);
        assert_eq!(value_of(&menu, &s, "Volume"), "0 %");
        menu.focus = Focus::Button(1);
        menu.click(middle(l.rows[0]), SIZE, M, &mut s, false);
        assert_eq!(value_of(&menu, &s, "Volume"), "0 %");
        // Apply's button.
        menu.click(middle(l.buttons[0]), SIZE, M, &mut s, false);
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
    fn the_layout_fits_the_window_from_900p_to_4k() {
        let s = settings();
        for size in [
            [1600.0, 900.0],
            SIZE,
            [2560.0, 1440.0],
            [3840.0, 2160.0],
            [1280.0, 720.0],
        ] {
            for tab in Tab::ALL {
                let menu = menu_on(tab);
                let l = menu.layout(size, M, &s);
                // The rows end above the description, the buttons above the hint, all inside.
                let last = l.rows.last().copied().unwrap_or_default();
                assert!(last[3] <= l.description[1] + 1e-3, "{size:?} {tab:?}");
                assert!(l.description[3] <= l.buttons[0][1] + 1e-3);
                assert!(
                    l.buttons
                        .iter()
                        .all(|b| b[3] <= l.hint.0 && b[0] >= l.column[0])
                );
                assert!(l.hint.0 + l.hint.1 <= size[1]);
                assert!(l.column[0] >= 0.0 && l.column[2] <= size[0]);
                for w in l.rows.windows(2) {
                    assert!(w[0][3] <= w[1][1] + 1e-3);
                }
                // Readable: about 23 pixels at 1080p, twice at 4K.
                assert!(l.px >= 13.0, "{size:?}: {}", l.px);
            }
        }
        let l = menu_on(Tab::Graphics).layout(SIZE, M, &s);
        assert!((l.px - 1080.0 / 46.0).abs() < 0.01, "{}", l.px);
        let l4 = menu_on(Tab::Graphics).layout([3840.0, 2160.0], M, &s);
        assert!((l4.px - 2.0 * l.px).abs() < 0.02);
    }

    #[test]
    fn text_wraps_between_words() {
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert!(wrap("", 9).is_empty());
    }
}

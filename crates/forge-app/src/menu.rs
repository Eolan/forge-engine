//! The game's menu on Esc (#352, the owner on 2026-10-22): resume, settings and quit, over the
//! picture as the HUD's quads (#328), with glyphs of its own, which [`crate::Context::hud_text`]
//! lends to the games too.
//!
//! - **Esc** opens it, and on its first page closes it again (resume); on the settings page it
//!   goes back.
//! - **The keys:** Up and Down (or W and S) choose, Enter or Space takes, Left and Right (or A
//!   and D) change a setting. The mouse chooses by hovering and takes by a click.
//! - **A solo game pauses behind it** ([`crate::Context::paused`]); a networked one
//!   ([`crate::Demo::networked`]) runs on.
//! - **The settings page** shows a few of the console's settings (D-069) and changes them as the
//!   console does, for the run; those the demo did not register are left out.

use std::path::Path;
use std::sync::Arc;

use forge_core::settings::{Kind, Settings, Source, Value};
use forge_gpu::{Device, GraphImage, ImageDesc, Result, vk};
use winit::keyboard::KeyCode;

use crate::hud::HudQuad;
use crate::overlay::{FontAtlas, GLYPHS, rasterize_builtin, rasterize_ttf};

/// The settings the page shows, in its order: the console's name, the label, and words for an
/// integer's values.
const SETTINGS: [(&str, &str, &[&str]); 4] = [
    ("r.aa", "Anti-aliasing", &[]),
    ("snd.volume", "Volume", &[]),
    ("app.mouse_look", "Mouse look", &[]),
    ("r.display_info", "Frame info", &["off", "on", "verbose"]),
];

/// The main page's entries.
const MAIN: [&str; 3] = ["Resume", "Settings", "Quit"];

/// The em sizes the glyphs are rasterised at, pixels: text takes the smallest at or above its
/// size, so it is drawn at 0.5 to 1.2 of an atlas.
const ATLAS_PX: [f32; 2] = [26.0, 56.0];
/// Empty pixels round each glyph in an atlas, so the linear filter never reads a neighbour.
const PAD: u32 = 2;

/// The colours, linear with straight alpha.
const DIM: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
const PANEL: [f32; 4] = [0.012, 0.014, 0.018, 0.86];
const BAR: [f32; 4] = [1.0, 1.0, 1.0, 0.08];
const ACCENT: [f32; 4] = [0.95, 0.62, 0.22, 1.0];
const TEXT: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
const CHOSEN: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const TITLE: [f32; 4] = [0.95, 0.62, 0.22, 1.0];
const HINT: [f32; 4] = [0.40, 0.40, 0.40, 1.0];

/// What the menu asks of the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// It closed: the game goes on.
    Resume,
    /// Close the window.
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Main,
    Settings,
}

/// The menu's state.
#[derive(Debug, Default)]
pub struct Menu {
    open: bool,
    page: Page,
    selected: usize,
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
#[derive(Debug)]
struct Layout {
    /// The entries' text size.
    px: f32,
    /// The panel.
    panel: [f32; 4],
    /// The title's top, and its size.
    title: (f32, f32),
    /// Each entry's rectangle, its label and its value.
    rows: Vec<([f32; 4], String, Option<String>)>,
    /// The hint's top, and its size.
    hint: (f32, f32),
}

impl Menu {
    /// Whether it shows.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Shows it on its first page, the first entry chosen.
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            ..Self::default()
        };
    }

    /// Hides it.
    pub fn close(&mut self) {
        self.open = false;
    }

    /// A key went down (or repeats): what it asks of the shell.
    pub fn key(&mut self, code: KeyCode, settings: &mut Settings) -> Option<Action> {
        let count = self.entries(settings).len();
        match code {
            KeyCode::Escape => match self.page {
                Page::Main => {
                    self.close();
                    return Some(Action::Resume);
                }
                Page::Settings => self.back(),
            },
            KeyCode::ArrowUp | KeyCode::KeyW => self.selected = (self.selected + count - 1) % count,
            KeyCode::ArrowDown | KeyCode::KeyS => self.selected = (self.selected + 1) % count,
            KeyCode::ArrowLeft | KeyCode::KeyA => self.change(settings, -1),
            KeyCode::ArrowRight | KeyCode::KeyD => self.change(settings, 1),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => return self.take(settings),
            _ => {}
        }
        None
    }

    /// The cursor at `at` in a window of `size` pixels: the entry under it is chosen.
    pub fn hover(&mut self, at: [f32; 2], size: [f32; 2], m: Metrics, settings: &Settings) {
        if let Some(k) = self.under(at, size, m, settings) {
            self.selected = k;
        }
    }

    /// A click at `at`: the entry under it taken.
    pub fn click(
        &mut self,
        at: [f32; 2],
        size: [f32; 2],
        m: Metrics,
        settings: &mut Settings,
    ) -> Option<Action> {
        self.selected = self.under(at, size, m, settings)?;
        self.take(settings)
    }

    /// Its quads for a window of `size` pixels, over everything else: `paused` when the game
    /// holds behind it.
    pub fn draw(
        &self,
        font: &MenuFont,
        size: [f32; 2],
        settings: &Settings,
        paused: bool,
        out: &mut Vec<HudQuad>,
    ) {
        let m = font.metrics();
        let l = self.layout(size, m, settings);
        out.push(HudQuad::solid([0.0, 0.0, size[0], size[1]], DIM));
        out.push(HudQuad::solid(l.panel, PANEL));
        let centred = |text: &str, px: f32| {
            0.5 * (l.panel[0] + l.panel[2] - m.advance * px * text.len() as f32)
        };
        let title = if paused { "Paused" } else { "Menu" };
        font.text(
            out,
            [centred(title, l.title.1), l.title.0],
            l.title.1,
            TITLE,
            title,
        );
        let margin = 2.0 * m.advance * l.px;
        for (k, (rect, label, value)) in l.rows.iter().enumerate() {
            let chosen = k == self.selected;
            let y = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * l.px);
            if chosen {
                out.push(HudQuad::solid(*rect, BAR));
                out.push(HudQuad::solid(
                    [rect[0], rect[1], rect[0] + 0.12 * l.px, rect[3]],
                    ACCENT,
                ));
            }
            let color = if chosen { CHOSEN } else { TEXT };
            font.text(out, [rect[0] + margin, y], l.px, color, label);
            if let Some(value) = value {
                let shown = if chosen {
                    format!("< {value} >")
                } else {
                    value.clone()
                };
                let x = rect[2] - margin - m.advance * l.px * shown.len() as f32;
                font.text(out, [x, y], l.px, color, &shown);
            }
        }
        let hint = match (self.page, paused) {
            (Page::Main, true) => "Esc: back to the game",
            (Page::Main, false) => "The game goes on behind the menu",
            (Page::Settings, _) => "Left and Right change, Esc goes back",
        };
        font.text(
            out,
            [centred(hint, l.hint.1), l.hint.0],
            l.hint.1,
            HINT,
            hint,
        );
    }

    /// The entries of the page: labels, and the settings' values.
    fn entries(&self, settings: &Settings) -> Vec<(String, Option<String>)> {
        match self.page {
            Page::Main => MAIN.iter().map(|&e| (e.to_owned(), None)).collect(),
            Page::Settings => shown(settings)
                .map(|(name, label, words)| {
                    (label.to_owned(), Some(value_text(settings, name, words)))
                })
                .chain(std::iter::once(("Back".to_owned(), None)))
                .collect(),
        }
    }

    /// Takes the chosen entry.
    fn take(&mut self, settings: &mut Settings) -> Option<Action> {
        match self.page {
            Page::Main => match self.selected {
                0 => {
                    self.close();
                    Some(Action::Resume)
                }
                1 => {
                    self.page = Page::Settings;
                    self.selected = 0;
                    None
                }
                _ => Some(Action::Quit),
            },
            Page::Settings => {
                if self.selected >= shown(settings).count() {
                    self.back();
                } else {
                    self.change(settings, 1);
                }
                None
            }
        }
    }

    /// Back to the first page, its settings entry chosen.
    fn back(&mut self) {
        self.page = Page::Main;
        self.selected = 1;
    }

    /// Steps the chosen setting by `dir`: a switch flipped, a choice the next or the one before
    /// (round), a number by one or by a tenth of its range (kept in it).
    fn change(&mut self, settings: &mut Settings, dir: i64) {
        if self.page != Page::Settings {
            return;
        }
        let Some((name, ..)) = shown(settings).nth(self.selected) else {
            return;
        };
        let Some(info) = settings.info(name) else {
            return;
        };
        let text = match (&info.kind, info.value) {
            (_, Value::Bool(b)) => u8::from(!b).to_string(),
            (Kind::Int { min, max }, Value::Int(i)) => (i + dir).clamp(*min, *max).to_string(),
            (Kind::Float { min, max }, Value::Float(f)) => {
                let step = (max - min) / 10.0;
                let steps = ((f - min) / step).round() + dir as f64;
                format!("{:.3}", (min + steps * step).clamp(*min, *max))
            }
            (Kind::Choice(choices), Value::Choice(i)) => {
                let n = choices.len() as i64;
                choices[(i as i64 + dir).rem_euclid(n) as usize].to_owned()
            }
            _ => return,
        };
        if let Err(e) = settings.set_text(name, &text, Source::Console) {
            tracing::warn!("the menu's {name}: {e}");
        }
    }

    /// The entry under `at`, if any.
    fn under(
        &self,
        at: [f32; 2],
        size: [f32; 2],
        m: Metrics,
        settings: &Settings,
    ) -> Option<usize> {
        self.layout(size, m, settings)
            .rows
            .iter()
            .position(|(r, ..)| at[0] >= r[0] && at[0] < r[2] && at[1] >= r[1] && at[1] < r[3])
    }

    /// The panel in the window's middle: the title, the entries a line each, the hint.
    fn layout(&self, size: [f32; 2], m: Metrics, settings: &Settings) -> Layout {
        let px = (size[1] / 22.0).clamp(16.0, 60.0);
        let (title_px, hint_px) = (px * 1.3, (px * 0.5).max(12.0));
        let line = px * 1.7;
        let entries = self.entries(settings);
        let labels = entries.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
        // A value is drawn with its arrows when chosen: "< DLAA >".
        let values = entries
            .iter()
            .filter_map(|(_, v)| v.as_ref().map(|v| v.len() + 4))
            .max()
            .map_or(0, |v| v + 3);
        let cols = (labels + values + 4).max(16) as f32;
        let width = m.advance * px * cols;
        let height = title_px * 2.4 + line * entries.len() as f32 + hint_px * 3.0;
        let (left, top) = (0.5 * (size[0] - width), 0.5 * (size[1] - height));
        let first = top + title_px * 2.2;
        let rows = entries
            .into_iter()
            .enumerate()
            .map(|(k, (label, value))| {
                let y = first + line * k as f32;
                ([left, y, left + width, y + line], label, value)
            })
            .collect::<Vec<_>>();
        let bottom = top + height;
        Layout {
            px,
            panel: [left, top, left + width, bottom],
            title: (top + title_px * 0.6, title_px),
            rows,
            hint: (bottom - hint_px * 2.2, hint_px),
        }
    }
}

/// The page's settings the demo registered and this build shows.
fn shown(
    settings: &Settings,
) -> impl Iterator<Item = (&'static str, &'static str, &'static [&'static str])> + '_ {
    SETTINGS
        .into_iter()
        .filter(|(name, ..)| settings.info(name).is_some_and(|i| i.visible()))
}

/// A setting's value as the page shows it: a switch on or off, a level in per cent, a choice's
/// name (in capitals, but off), an integer by its word.
fn value_text(settings: &Settings, name: &str, words: &[&str]) -> String {
    let Some(info) = settings.info(name) else {
        return String::new();
    };
    match (&info.kind, info.value) {
        (_, Value::Bool(b)) => if b { "on" } else { "off" }.to_owned(),
        (Kind::Float { min, max }, Value::Float(f)) if *min == 0.0 && *max == 1.0 => {
            format!("{:.0} %", f * 100.0)
        }
        (_, Value::Int(i)) => usize::try_from(i)
            .ok()
            .and_then(|i| words.get(i))
            .map_or_else(|| i.to_string(), |w| (*w).to_owned()),
        (_, Value::Choice(_)) => {
            let text = info.value_text();
            if text == "off" {
                text
            } else {
                text.to_uppercase()
            }
        }
        _ => info.value_text(),
    }
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
    use forge_core::settings::Spec;

    const M: Metrics = Metrics {
        advance: 0.6,
        height: 1.3,
    };
    const SIZE: [f32; 2] = [1600.0, 900.0];

    fn settings() -> Settings {
        let mut s = Settings::new();
        s.add(Spec::float("snd.volume", 1.0, 0.0..=1.0, "volume"));
        s.add(Spec::bool("app.mouse_look", false, "look"));
        s.add(Spec::int("r.display_info", 0, 0..=2, "info"));
        s
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
    }

    #[test]
    fn the_settings_page_steps_the_registered_settings_and_goes_back() {
        let mut s = settings();
        let mut menu = Menu::default();
        menu.open();
        menu.key(KeyCode::ArrowDown, &mut s);
        assert_eq!(menu.key(KeyCode::Enter, &mut s), None);
        // r.aa is not registered here: the page shows the three others and Back.
        let labels: Vec<String> = menu.entries(&s).into_iter().map(|(l, _)| l).collect();
        assert_eq!(labels, ["Volume", "Mouse look", "Frame info", "Back"]);
        menu.key(KeyCode::ArrowLeft, &mut s);
        menu.key(KeyCode::ArrowLeft, &mut s);
        assert_eq!(
            s.info("snd.volume").map(|i| i.value),
            Some(Value::Float(0.8))
        );
        assert_eq!(menu.entries(&s)[0].1.as_deref(), Some("80 %"));
        menu.key(KeyCode::ArrowDown, &mut s);
        menu.key(KeyCode::Enter, &mut s);
        assert_eq!(
            s.info("app.mouse_look").map(|i| i.value),
            Some(Value::Bool(true))
        );
        menu.key(KeyCode::ArrowDown, &mut s);
        menu.key(KeyCode::ArrowRight, &mut s);
        menu.key(KeyCode::ArrowRight, &mut s);
        menu.key(KeyCode::ArrowRight, &mut s);
        assert_eq!(menu.entries(&s)[2].1.as_deref(), Some("verbose"));
        // Esc goes back to the first page, on Settings; Esc again resumes.
        assert_eq!(menu.key(KeyCode::Escape, &mut s), None);
        assert_eq!(menu.entries(&s).len(), 3);
        assert_eq!(menu.key(KeyCode::Escape, &mut s), Some(Action::Resume));
    }

    #[test]
    fn the_mouse_chooses_by_hovering_and_takes_by_a_click() {
        let mut s = settings();
        let mut menu = Menu::default();
        menu.open();
        let rows = menu.layout(SIZE, M, &s).rows;
        let middle = |r: [f32; 4]| [0.5 * (r[0] + r[2]), 0.5 * (r[1] + r[3])];
        menu.hover(middle(rows[2].0), SIZE, M, &s);
        assert_eq!(menu.selected, 2);
        // Off every entry, the choice stays.
        menu.hover([1.0, 1.0], SIZE, M, &s);
        assert_eq!(menu.selected, 2);
        assert_eq!(menu.click([1.0, 1.0], SIZE, M, &mut s), None);
        assert_eq!(
            menu.click(middle(rows[0].0), SIZE, M, &mut s),
            Some(Action::Resume)
        );
        // Each entry in the window, below the title, one under another.
        for w in rows.windows(2) {
            assert!(w[0].0[3] <= w[1].0[1] + 1e-3);
        }
        assert!(
            rows.iter()
                .all(|(r, ..)| r[0] >= 0.0 && r[2] <= SIZE[0] && r[3] <= SIZE[1])
        );
    }
}

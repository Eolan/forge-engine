//! The Carbon look, the default (#377, the owner on 2026-10-31: "with the theme of the project in
//! the directory obs-streaming-project"): the owner's streaming overlays' dark, ion blue and
//! amber, Orbitron for the titles, Rajdhani for the text and JetBrains Mono for the values
//! ([`Face`]), the titles, tabs, values and buttons in capitals, dotted leaders from a label to
//! its value, a glow round what is lit; without the overlays' corner brackets, the blue bar down
//! the column's side and the grid behind (the owner). Its text is placed by its baselines; the
//! glass behind it is tinted to match (`shaders/menu_glass.slang`).

use forge_core::settings::Settings;

use super::items::{self, Row, Tab};
use super::{
    BUTTONS, Behind, COLUMNS, DIALOG, Face, Focus, HINT, Layout, MAIN, Measure, Menu, MenuFont,
    Page, Status, Style, frame, other, wrap,
};
use crate::hud::HudQuad;

/// The small line over the titles.
const CALL: &str = "FORGE";

/// The Carbon HUD's colours (`overlays/_hud.css`), sRGB: its accent, what waits (a change, a
/// restart), the text, the quieter text and the dark.
const ION: u32 = 0x4FD8FF;
const AMBER: u32 = 0xFFB454;
const INK: u32 = 0xE7ECEF;
const DIM: u32 = 0x7C8A93;
const VOID: u32 = 0x0A0B0E;

/// Over the whole window where the glass cannot be drawn, linear.
const VEIL: [f32; 4] = [0.003, 0.0034, 0.0044, 0.86];

/// The sizes of the tabs', the buttons' and the values' text, in the body text's, and the
/// spacing of their letters, ems; the buttons' height in the body text's size.
const TAB_PX: f32 = 0.95;
const TAB_SPACING: f32 = 0.16;
const BUTTON_PX: f32 = 0.85;
const BUTTON_SPACING: f32 = 0.18;
const BUTTON_H: f32 = 1.9;
const VALUE_PX: f32 = 0.8;
const VALUE_SPACING: f32 = 0.12;

/// The sRGB value `c` (0 to 1) in linear light.
fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// How the page's colours become the HUD's: it blends in linear light (SDR, scRGB), or on PQ's
/// codes (HDR10), which are perceptual as the page's sRGB is.
#[derive(Clone, Copy)]
struct Blend {
    pq: bool,
}

impl Blend {
    /// `hex` at the translucency `alpha` a web page gives it (blending in sRGB). In linear light,
    /// a light colour at the alpha that looks the same over the dark (smaller), a dark veil at the
    /// one that darkens as much (larger); on PQ's codes, the page's alpha as it is.
    fn color(self, hex: u32, alpha: f32) -> [f32; 4] {
        let srgb = [hex >> 16, hex >> 8, hex].map(|c| (c & 255) as f32 / 255.0);
        let peak = srgb.iter().copied().fold(0.0, f32::max);
        let alpha = alpha.clamp(0.0, 1.0);
        let a = if alpha >= 1.0 || self.pq {
            alpha
        } else if peak < 0.1 {
            1.0 - (1.0 - alpha).powf(2.2)
        } else {
            to_linear(alpha * peak) / to_linear(peak)
        };
        let [r, g, b] = srgb.map(to_linear);
        [r, g, b, a]
    }
}

/// The column down the window's middle: the call line and the title, the tabs, the rows, the
/// description, the buttons and the hint; the text sized to the window and to fit it.
pub(super) fn layout(
    menu: &Menu,
    size: [f32; 2],
    measure: &dyn Measure,
    settings: &Settings,
) -> Layout {
    let m = measure.metrics();
    let [w, h] = size;
    let rows = match menu.page {
        Page::Main => Vec::new(),
        Page::Settings => menu.rows(settings),
    };
    // About 23 pixels at 1080p, 47 at 4K; smaller when the rows or the width need it.
    let lines = match menu.page {
        Page::Main => 20.0,
        Page::Settings => 18.8 + 1.55 * rows.len() as f32,
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
    let small = (0.62 * px).max(11.0);
    let hint_top = h - 1.4 * px - small;
    let mut l = Layout {
        px,
        column: [left, 0.0, left + width, h],
        inner,
        hint: (hint_top, small),
        ..Layout::default()
    };
    let row_rects = |first: f32, line: f32, n: usize| -> Vec<[f32; 4]> {
        (0..n)
            .map(|k| {
                let y = first + line * k as f32;
                [left, y, left + width, y + line]
            })
            .collect()
    };
    match menu.page {
        Page::Main => {
            let line = 2.0 * px;
            let first = 0.5 * (h - line * MAIN.len() as f32) + 0.6 * px;
            let title_base = first - 1.3 * px;
            l.call = ([inner[0], title_base - 2.2 * px], small);
            l.title = ([inner[0], title_base], 1.9 * px);
            l.title_rule = [
                inner[0],
                title_base + 0.5 * px,
                inner[0] + 12.0 * px,
                title_base + 0.58 * px,
            ];
            l.rows = row_rects(first, line, MAIN.len());
            l.px = 1.25 * px;
        }
        Page::Settings => {
            l.call = ([inner[0], top + 0.7 * px], small);
            let title_base = top + 2.55 * px;
            l.title = ([inner[0], title_base], 1.35 * px);
            l.title_rule = [
                inner[0],
                title_base + 0.55 * px,
                inner[0] + 11.0 * px,
                title_base + 0.63 * px,
            ];
            let tabs_top = title_base + 1.2 * px;
            let tab_h = 1.9 * px;
            let style = Style::new(Face::Strong, TAB_PX * px, TAB_SPACING);
            let mut x = inner[0];
            for tab in Tab::ALL {
                let tw = measure.width(style, &tab.label().to_uppercase());
                l.tabs.push([x, tabs_top, x + tw, tabs_top + tab_h]);
                x += tw + 2.0 * m.advance * px;
            }
            l.rule = tabs_top + tab_h;
            l.rows = row_rects(l.rule + 0.7 * px, 1.55 * px, rows.len());
            let buttons_bottom = hint_top - 0.7 * px;
            l.buttons = buttons(measure, &BUTTONS, inner[1], buttons_bottom, px);
            let description_bottom = buttons_bottom - BUTTON_H * px - 0.8 * px;
            l.description = [
                inner[0],
                description_bottom - 4.9 * px,
                inner[1],
                description_bottom,
            ];
        }
    }
    if menu.dialog.is_some() {
        let dw = (46.0 * m.advance * px).min(0.96 * w);
        let dh = 7.6 * px;
        let (dx, dy) = (0.5 * (w - dw), 0.5 * (h - dh));
        let right = dx + dw - 1.6 * m.advance * px;
        let bottom = dy + dh - 1.1 * px;
        let buttons = buttons(measure, &DIALOG, right, bottom, px);
        l.dialog = Some(([dx, dy, dx + dw, dy + dh], buttons));
    }
    l
}

/// The menu's quads for a window of `size` pixels ([`Menu::draw`]).
pub(super) fn draw(
    menu: &Menu,
    font: &MenuFont,
    size: [f32; 2],
    settings: &Settings,
    behind: Behind,
    out: &mut Vec<HudQuad>,
) {
    let l = layout(menu, size, font, settings);
    let blend = Blend { pq: behind.pq };
    if !behind.glass {
        out.push(HudQuad::solid([0.0, 0.0, size[0], size[1]], VEIL));
    }
    // The column, darker than the glass, without edges.
    out.push(HudQuad::solid(l.column, blend.color(VOID, 0.55)));
    let (call, title) = match menu.page {
        Page::Main => (
            CALL.to_owned(),
            if behind.paused { "PAUSED" } else { "MENU" },
        ),
        Page::Settings => (format!("{CALL} // SYSTEM CONFIG"), "SETTINGS"),
    };
    let (at, px) = l.call;
    font.line(
        out,
        Style::new(Face::Mono, px, 0.42),
        at,
        blend.color(ION, 1.0),
        &call,
    );
    let (at, px) = l.title;
    font.line(
        out,
        Style::new(Face::Title, px, 0.12),
        at,
        blend.color(INK, 1.0),
        title,
    );
    fade(blend, l.title_rule, ION, 0.75, out);
    match menu.page {
        Page::Main => draw_main(menu, font, &l, blend, behind.paused, out),
        Page::Settings => draw_settings(menu, font, &l, blend, settings, out),
    }
    if let Some((panel, buttons)) = &l.dialog {
        draw_dialog(menu, font, &l, blend, *panel, buttons, out);
    }
}

fn draw_main(
    menu: &Menu,
    font: &MenuFont,
    l: &Layout,
    blend: Blend,
    paused: bool,
    out: &mut Vec<HudQuad>,
) {
    let entry = Style::new(Face::Strong, l.px, 0.14);
    for (k, rect) in l.rows.iter().enumerate() {
        let chosen = k == menu.selected;
        if chosen {
            focus_bar(blend, *rect, l.px, out);
        }
        let ink = blend.color(INK, if chosen { 1.0 } else { 0.72 });
        let at = [l.inner[0], baseline(*rect, l.px)];
        font.line(out, entry, at, ink, &MAIN[k].to_uppercase());
    }
    let hint = if paused {
        "ESC  BACK TO THE GAME"
    } else {
        "THE GAME GOES ON BEHIND THE MENU"
    };
    let (top, px) = l.hint;
    let at = [l.inner[0], top + 0.75 * px];
    font.line(
        out,
        Style::new(Face::Mono, px, 0.12),
        at,
        blend.color(DIM, 1.0),
        hint,
    );
}

fn draw_settings(
    menu: &Menu,
    font: &MenuFont,
    l: &Layout,
    blend: Blend,
    settings: &Settings,
    out: &mut Vec<HudQuad>,
) {
    let px = l.px;
    let hair = (0.05 * px).round().max(1.0);
    let small = Style::new(Face::Mono, l.hint.1, 0.3);
    // The tabs, the chosen one bright and underlined, a glow round its line.
    let tab = Style::new(Face::Strong, TAB_PX * px, TAB_SPACING);
    for (k, rect) in l.tabs.iter().enumerate() {
        let on = Tab::ALL[k] == menu.tab;
        let ink = blend.color(if on { INK } else { DIM }, 1.0);
        let at = [rect[0], baseline(*rect, tab.px)];
        font.line(out, tab, at, ink, &Tab::ALL[k].label().to_uppercase());
        if on {
            let [x0, _, x1, y] = *rect;
            let line = (0.09 * px).round().max(2.0);
            out.push(HudQuad::solid(
                [x0, y - 0.7 * px, x1, y - line],
                blend.color(ION, 0.05),
            ));
            out.push(HudQuad::solid([x0, y - line, x1, y], blend.color(ION, 1.0)));
            out.push(HudQuad::solid(
                [x0, y, x1, y + 0.2 * px],
                blend.color(ION, 0.12),
            ));
        }
    }
    let rule = [l.inner[0], l.rule, l.inner[1], l.rule + hair];
    out.push(HudQuad::solid(rule, blend.color(ION, 0.15)));
    let rows = menu.rows(settings);
    let focus = menu.focused(&rows);
    let value = |n: &str| menu.draft.value(settings, n);
    let label = Style::new(Face::Text, px, 0.03);
    let shown_style = Style::new(Face::Mono, VALUE_PX * px, VALUE_SPACING);
    let header = Style::new(Face::Mono, l.hint.1, 0.42);
    for (k, (row, rect)) in rows.iter().zip(&l.rows).enumerate() {
        let focused = focus == Focus::Row(k);
        let base = baseline(*rect, px);
        let label_ink = |off: bool| match (off, focused) {
            (true, _) => blend.color(DIM, 0.42),
            (false, true) => blend.color(INK, 1.0),
            (false, false) => blend.color(INK, 0.8),
        };
        match row {
            Row::Header(title) => {
                let hb = baseline(*rect, header.px) + 0.12 * px;
                font.line(out, header, [l.inner[0], hb], blend.color(DIM, 1.0), title);
                let x = l.inner[0] + font.width(header, title) + 0.8 * px;
                let mid = (hb - 0.35 * header.px).round();
                let line = [x, mid, l.inner[1], mid + hair];
                out.push(HudQuad::solid(line, blend.color(ION, 0.15)));
            }
            Row::Setting(s) => {
                if focused {
                    focus_bar(blend, *rect, px, out);
                }
                let off = s.off.is_some();
                font.line(out, label, [l.inner[0], base], label_ink(off), s.label);
                let mut end = l.inner[0] + font.width(label, s.label);
                if s.names().iter().any(|n| items::restart(settings, n)) {
                    let x = end + 0.6 * px;
                    font.line(out, small, [x, base], blend.color(AMBER, 0.85), "RESTART");
                    end = x + font.width(small, "RESTART");
                }
                let changed = s.names().iter().any(|n| menu.draft.changes(settings, n));
                let shown = match (&s.shown_off, off) {
                    (Some(text), true) => text.clone(),
                    _ => match s.current(&value) {
                        Some(i) => s.choices[i].label.clone(),
                        None => other(settings, s),
                    },
                }
                // In capitals, but a size's "x" (1920 x 1080) stays small.
                .to_uppercase()
                .replace(" X ", " x ");
                let shown = if focused && !off {
                    format!("< {shown} >")
                } else {
                    shown
                };
                // A change waiting for Apply in amber.
                let ink = match (off, changed, focused) {
                    (true, _, _) => blend.color(DIM, 0.42),
                    (false, true, _) => blend.color(AMBER, 1.0),
                    (false, false, true) => blend.color(ION, 1.0),
                    (false, false, false) => blend.color(DIM, 1.0),
                };
                let x = l.inner[1] - font.width(shown_style, &shown);
                font.line(out, shown_style, [x, base], ink, &shown);
                leaders(
                    blend,
                    l.inner[0],
                    end + 0.5 * px,
                    x - 0.5 * px,
                    base,
                    px,
                    out,
                );
            }
            Row::Action {
                label: text, off, ..
            } => {
                if focused {
                    focus_bar(blend, *rect, px, out);
                }
                font.line(
                    out,
                    label,
                    [l.inner[0], base],
                    label_ink(off.is_some()),
                    text,
                );
                let end = l.inner[0] + font.width(label, text);
                let x = l.inner[1] - font.width(shown_style, "OPEN >");
                let ink = if off.is_some() {
                    blend.color(DIM, 0.42)
                } else {
                    blend.color(ION, if focused { 1.0 } else { 0.7 })
                };
                font.line(out, shown_style, [x, base], ink, "OPEN >");
                leaders(
                    blend,
                    l.inner[0],
                    end + 0.5 * px,
                    x - 0.5 * px,
                    base,
                    px,
                    out,
                );
            }
        }
    }
    // What the focused row does, and how it applies: at most four lines.
    let d = l.description;
    out.push(HudQuad::solid(
        [d[0], d[1], d[2], d[1] + hair],
        blend.color(ION, 0.16),
    ));
    let text = Style::new(Face::Text, 0.9 * px, 0.0);
    let step = 1.3 * text.px;
    let mut y = d[1] + 0.6 * px + 0.75 * text.px;
    let (description, status) = menu.describe(settings, &rows, focus);
    let measure = |s: &str| font.width(text, s);
    let lines = wrap(&description, d[2] - d[0], measure);
    let shown = lines.len().min(3);
    for line in &lines[..shown] {
        font.line(out, text, [d[0], y], blend.color(INK, 0.72), line);
        y += step;
    }
    match status {
        Some(Status::Live) => {
            font.line(
                out,
                small,
                [d[0], y],
                blend.color(ION, 1.0),
                "APPLIES AT ONCE",
            );
        }
        Some(Status::Later) => {
            let ink = blend.color(AMBER, 1.0);
            font.line(out, small, [d[0], y], ink, "APPLIES AFTER YOU RESTART");
        }
        Some(Status::Off(why)) => {
            for line in wrap(&why, d[2] - d[0], measure).iter().take(4 - shown) {
                font.line(out, text, [d[0], y], blend.color(AMBER, 0.9), line);
                y += step;
            }
        }
        None => {}
    }
    // The buttons; Apply lit, a glow round it, only with something to apply.
    let dirty = menu.draft.is_dirty(settings);
    for (k, rect) in l.buttons.iter().enumerate() {
        let lit = Lit {
            focused: focus == Focus::Button(k),
            live: k != 0 || dirty,
            go: k == 0 && dirty,
        };
        button(font, blend, *rect, BUTTONS[k], px, lit, out);
    }
    let (hint, ink) = match &menu.notice {
        Some(notice) => (notice.to_uppercase(), blend.color(ION, 1.0)),
        None => (HINT.to_uppercase(), blend.color(DIM, 1.0)),
    };
    let (top, size) = l.hint;
    let at = [l.inner[0], top + 0.75 * size];
    // Smaller where a long notice, or the smallest windows' floor size, would pass the column.
    let fit = (l.inner[1] - l.inner[0]) / font.width(Style::new(Face::Mono, size, 0.1), &hint);
    let style = Style::new(Face::Mono, size * fit.clamp(0.5, 1.0), 0.1);
    font.line(out, style, at, ink, &hint);
}

fn draw_dialog(
    menu: &Menu,
    font: &MenuFont,
    l: &Layout,
    blend: Blend,
    panel: [f32; 4],
    buttons: &[[f32; 4]],
    out: &mut Vec<HudQuad>,
) {
    let px = l.px;
    let hair = (0.05 * px).round().max(1.0);
    out.push(HudQuad::solid(panel, blend.color(VOID, 0.92)));
    frame(panel, hair, blend.color(ION, 0.3), out);
    let x = panel[0] + 1.6 * font.metrics().advance * px;
    let title = Style::new(Face::Title, 0.9 * px, 0.1);
    let at = [x, panel[1] + 1.9 * px];
    font.line(out, title, at, blend.color(INK, 1.0), "APPLY THE CHANGES?");
    let text = Style::new(Face::Text, 0.9 * px, 0.0);
    let at = [x, panel[1] + 3.4 * px];
    let line = "Some settings changed and are not applied yet.";
    font.line(out, text, at, blend.color(INK, 0.72), line);
    for (k, rect) in buttons.iter().enumerate() {
        let lit = Lit {
            focused: menu.dialog == Some(k),
            live: true,
            go: false,
        };
        button(font, blend, *rect, DIALOG[k], px, lit, out);
    }
}

/// Buttons labelled `labels` side by side, the last one's right edge at `right`, their bottoms
/// at `bottom`.
fn buttons(
    measure: &dyn Measure,
    labels: &[&str],
    right: f32,
    bottom: f32,
    px: f32,
) -> Vec<[f32; 4]> {
    let advance = measure.metrics().advance * px;
    let style = Style::new(Face::Strong, BUTTON_PX * px, BUTTON_SPACING);
    let mut x = right;
    let mut rects: Vec<[f32; 4]> = labels
        .iter()
        .rev()
        .map(|label| {
            let w = measure.width(style, &label.to_uppercase()) + 3.0 * advance;
            let r = [x - w, bottom - BUTTON_H * px, x, bottom];
            x -= w + 1.2 * advance;
            r
        })
        .collect();
    rects.reverse();
    rects
}

/// How a button is lit: `focused`, `live` when it does something now, `go` for Apply with
/// changes to apply (the accent, and a glow).
#[derive(Clone, Copy)]
struct Lit {
    focused: bool,
    live: bool,
    go: bool,
}

/// A button: its label centred, a hairline round it.
fn button(
    font: &MenuFont,
    blend: Blend,
    r: [f32; 4],
    label: &str,
    px: f32,
    lit: Lit,
    out: &mut Vec<HudQuad>,
) {
    let hair = (0.05 * px).round().max(1.0);
    if lit.go {
        let band = 0.15 * px;
        frame(grow(r, band), band, blend.color(ION, 0.1), out);
        frame(grow(r, 2.0 * band), band, blend.color(ION, 0.04), out);
    }
    if lit.focused {
        out.push(HudQuad::solid(r, blend.color(ION, 0.1)));
    }
    let border = match (lit.focused || lit.go, lit.live) {
        (true, _) => blend.color(ION, 1.0),
        (false, true) => blend.color(ION, 0.28),
        (false, false) => blend.color(ION, 0.14),
    };
    frame(r, hair, border, out);
    let ink = match (lit.live, lit.go, lit.focused) {
        (false, _, _) => blend.color(DIM, 0.45),
        (true, true, _) => blend.color(ION, 1.0),
        (true, false, true) => blend.color(INK, 1.0),
        (true, false, false) => blend.color(INK, 0.75),
    };
    let style = Style::new(Face::Strong, BUTTON_PX * px, BUTTON_SPACING);
    let label = label.to_uppercase();
    let x = 0.5 * (r[0] + r[2] - font.width(style, &label));
    font.line(out, style, [x, baseline(r, style.px)], ink, &label);
}

/// `r` grown by `d` on every side.
fn grow(r: [f32; 4], d: f32) -> [f32; 4] {
    [r[0] - d, r[1] - d, r[2] + d, r[3] + d]
}

/// The baseline of text `px` in size in `r`, its capitals centred.
fn baseline(r: [f32; 4], px: f32) -> f32 {
    0.5 * (r[1] + r[3]) + 0.35 * px
}

/// The focused row's mark: a band of the accent, and its edge. An even band: one fading across
/// the row broke into steps in 8 bits, which the HUD's quads cannot dither.
fn focus_bar(blend: Blend, r: [f32; 4], px: f32, out: &mut Vec<HudQuad>) {
    out.push(HudQuad::solid(r, blend.color(ION, 0.08)));
    let edge = (0.09 * px).round().max(2.0);
    out.push(HudQuad::solid(
        [r[0], r[1], r[0] + edge, r[3]],
        blend.color(ION, 1.0),
    ));
}

/// `r` filled with `hex` fading from `alpha` at its left to nothing at its right, as a page's
/// gradient, in bands.
fn fade(blend: Blend, r: [f32; 4], hex: u32, alpha: f32, out: &mut Vec<HudQuad>) {
    const BANDS: usize = 40;
    let w = (r[2] - r[0]) / BANDS as f32;
    for k in 0..BANDS {
        let t = (k as f32 + 0.5) / BANDS as f32;
        let x = r[0] + w * k as f32;
        out.push(HudQuad::solid(
            [x, r[1], x + w, r[3]],
            blend.color(hex, alpha * (1.0 - t)),
        ));
    }
}

/// The dotted line from a label to its value, `x0` to `x1` on the baseline `y`, its dots on a
/// grid from `origin` so that they line up from row to row, on whole pixels so that each is the
/// same.
fn leaders(blend: Blend, origin: f32, x0: f32, x1: f32, y: f32, px: f32, out: &mut Vec<HudQuad>) {
    let step = (0.3 * px).round();
    let dot = (0.07 * px).round().max(1.0);
    if step < 2.0 || x1 <= x0 {
        return;
    }
    let dots = blend.color(DIM, 0.38);
    let (origin, y) = (origin.round(), y.round());
    let mut x = origin + ((x0 - origin) / step).ceil() * step;
    while x + dot <= x1 {
        out.push(HudQuad::solid([x, y - dot, x + dot, y], dots));
        x += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINEAR: Blend = Blend { pq: false };

    #[test]
    fn web_alphas_look_the_same_blended_in_linear_light_or_on_pq_codes() {
        assert_eq!(LINEAR.color(ION, 1.0)[3], 1.0);
        // The ion blue at half over black: the sRGB page's mix of its brightest channel, in
        // linear light.
        let [_, _, b, a] = LINEAR.color(ION, 0.5);
        assert!((b * a - to_linear(0.5)).abs() < 1e-5, "{}", b * a);
        // A dark veil darkens as much as on the page: more alpha in linear light.
        assert!(LINEAR.color(VOID, 0.55)[3] > 0.55);
        // On PQ's codes, perceptual as the page's are: as the page has it.
        assert_eq!(Blend { pq: true }.color(ION, 0.16)[3], 0.16);
    }
}

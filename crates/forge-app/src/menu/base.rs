//! The base look (`app.menu_theme base`): the menu's first one (#377), plainer, kept for a game
//! that wants it (the owner, 2026-10-31: "keep the one you did before somewhere, it fits more the
//! base theme"). One monospaced face, grey text, an orange accent, the column's edges drawn.
//! Its text is placed by its cells' top left.

use forge_core::settings::Settings;

use super::items::{self, Row, Tab};
use super::{
    BUTTONS, Behind, COLUMNS, DIALOG, Focus, HINT, Layout, MAIN, Measure, Menu, MenuFont, Metrics,
    Page, Status, frame, other, wrap,
};
use crate::hud::HudQuad;

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
const HINT_INK: [f32; 4] = [0.40, 0.40, 0.40, 1.0];
const OFF: [f32; 4] = [0.20, 0.20, 0.20, 1.0];
const LATER: [f32; 4] = [0.80, 0.64, 0.36, 1.0];
const LIVE: [f32; 4] = [0.42, 0.66, 0.46, 1.0];
const DIALOG_PANEL: [f32; 4] = [0.018, 0.020, 0.026, 0.97];

/// The column down the window's middle: the title, the tabs, the rows, the description, the
/// buttons and the hint; the text sized to the window and to fit it.
pub(super) fn layout(menu: &Menu, size: [f32; 2], m: Metrics, settings: &Settings) -> Layout {
    let [w, h] = size;
    let rows = match menu.page {
        Page::Main => Vec::new(),
        Page::Settings => menu.rows(settings),
    };
    // About 23 pixels at 1080p, 47 at 4K; smaller when the rows or the width need it.
    let lines = match menu.page {
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
    match menu.page {
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
                let tw = m.cells(0.95 * px, tab.label());
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
            l.buttons = buttons(m, &BUTTONS, inner[1], buttons_bottom, px);
            let description_bottom = buttons_bottom - button_h - 0.8 * px;
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
        let buttons = buttons(m, &DIALOG, right, bottom, px);
        l.dialog = Some(([dx, dy, dx + dw, dy + dh], buttons));
    }
    l
}

/// Buttons labelled `labels` side by side, the last one's right edge at `right`, their bottoms
/// at `bottom`.
fn buttons(m: Metrics, labels: &[&str], right: f32, bottom: f32, px: f32) -> Vec<[f32; 4]> {
    let mut x = right;
    let mut rects: Vec<[f32; 4]> = labels
        .iter()
        .rev()
        .map(|label| {
            let bw = m.cells(px, label) + 3.0 * m.advance * px;
            let r = [x - bw, bottom - 1.9 * px, x, bottom];
            x -= bw + 1.2 * m.advance * px;
            r
        })
        .collect();
    rects.reverse();
    rects
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
    let l = layout(menu, size, font.metrics(), settings);
    if !behind.glass {
        out.push(HudQuad::solid([0.0, 0.0, size[0], size[1]], DIM));
    }
    let hair = (0.05 * l.px).max(1.0);
    out.push(HudQuad::solid(l.column, COLUMN));
    let [left, top, right, bottom] = l.column;
    out.push(HudQuad::solid([left, top, left + hair, bottom], EDGE));
    out.push(HudQuad::solid([right - hair, top, right, bottom], EDGE));
    match menu.page {
        Page::Main => draw_main(menu, font, &l, behind.paused, out),
        Page::Settings => draw_settings(menu, font, &l, settings, hair, out),
    }
    if let Some((panel, buttons)) = &l.dialog {
        draw_dialog(menu, font, &l, *panel, buttons, hair, out);
    }
}

fn draw_main(menu: &Menu, font: &MenuFont, l: &Layout, paused: bool, out: &mut Vec<HudQuad>) {
    let m = font.metrics();
    let title = if paused { "PAUSED" } else { "MENU" };
    font.text(out, l.title.0, l.title.1, TITLE, title);
    for (k, rect) in l.rows.iter().enumerate() {
        let chosen = k == menu.selected;
        if chosen {
            bar(*rect, l.px, out);
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
    font.text(out, [l.inner[0], l.hint.0], l.hint.1, HINT_INK, hint);
}

fn draw_settings(
    menu: &Menu,
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
        let on = Tab::ALL[k] == menu.tab;
        let y = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * 0.95 * px);
        let color = if on { CHOSEN } else { TEXT };
        font.text(out, [rect[0], y], 0.95 * px, color, Tab::ALL[k].label());
        if on {
            let line = [rect[0], rect[3] - 0.13 * px, rect[2], rect[3]];
            out.push(HudQuad::solid(line, ACCENT));
        }
    }
    let rule = [l.inner[0], l.rule, l.inner[1], l.rule + hair];
    out.push(HudQuad::solid(rule, RULE));
    let rows = menu.rows(settings);
    let focus = menu.focused(&rows);
    let value = |n: &str| menu.draft.value(settings, n);
    for (k, (row, rect)) in rows.iter().zip(&l.rows).enumerate() {
        let focused = focus == Focus::Row(k);
        let y = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * px);
        match row {
            Row::Header(title) => {
                let hp = 0.7 * px;
                let hy = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * hp) + 0.15 * px;
                font.text(out, [l.inner[0], hy], hp, HINT_INK, title);
                let x = l.inner[0] + m.cells(hp, title) + 1.2 * m.advance * px;
                let mid = hy + 0.55 * m.height * hp;
                out.push(HudQuad::solid([x, mid, l.inner[1], mid + hair], RULE));
            }
            Row::Setting(s) => {
                if focused {
                    bar(*rect, px, out);
                }
                let off = s.off.is_some();
                let color = match (off, focused) {
                    (true, _) => OFF,
                    (false, true) => CHOSEN,
                    (false, false) => TEXT,
                };
                font.text(out, [l.inner[0], y], px, color, s.label);
                if s.names().iter().any(|n| items::restart(settings, n)) {
                    let x = l.inner[0] + m.cells(px, s.label) + 1.5 * m.advance * px;
                    let tp = 0.7 * px;
                    let at = [x, y + 0.5 * m.height * (px - tp)];
                    font.text(out, at, tp, LATER, "restart");
                }
                let changed = s.names().iter().any(|n| menu.draft.changes(settings, n));
                let shown = match (&s.shown_off, off) {
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
                let x = l.inner[1] - m.cells(px, &shown);
                font.text(out, [x, y], px, vcolor, &shown);
            }
            Row::Action { label, off, .. } => {
                if focused {
                    bar(*rect, px, out);
                }
                let color = match (off.is_some(), focused) {
                    (true, _) => OFF,
                    (false, true) => CHOSEN,
                    (false, false) => TEXT,
                };
                font.text(out, [l.inner[0], y], px, color, label);
                let x = l.inner[1] - m.cells(px, "Open >  ");
                font.text(out, [x, y], px, color, "Open >  ");
            }
        }
    }
    // What the focused row does, and whether it applies at once.
    let d = l.description;
    out.push(HudQuad::solid([d[0], d[1], d[2], d[1] + hair], RULE));
    let dp = 0.8 * px;
    let mut y = d[1] + 0.6 * px;
    let line = 1.35 * dp;
    let (text, status) = menu.describe(settings, &rows, focus);
    for text_line in wrap(&text, d[2] - d[0], |s| m.cells(dp, s))
        .into_iter()
        .take(3)
    {
        font.text(out, [d[0], y], dp, TEXT, &text_line);
        y += line;
    }
    let status = status.map(|status| match status {
        Status::Live => ("Applies at once.".to_owned(), LIVE),
        Status::Later => ("Applies after you restart.".to_owned(), LATER),
        Status::Off(why) => (why, LATER),
    });
    if let Some((status, color)) = status {
        font.text(out, [d[0], y], dp, color, &status);
    }
    // The buttons; Apply bright only with something to apply.
    let dirty = menu.draft.is_dirty(settings);
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
        let tx = 0.5 * (rect[0] + rect[2] - m.cells(px, BUTTONS[k]));
        let ty = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * px);
        font.text(out, [tx, ty], px, color, BUTTONS[k]);
    }
    let (hint, color) = match &menu.notice {
        Some(notice) => (notice.as_str(), LIVE),
        None => (HINT, HINT_INK),
    };
    font.text(out, [l.inner[0], l.hint.0], l.hint.1, color, hint);
}

/// The chosen row's mark: a band, and the accent at its left.
fn bar(rect: [f32; 4], px: f32, out: &mut Vec<HudQuad>) {
    out.push(HudQuad::solid(rect, BAR));
    let edge = [rect[0], rect[1], rect[0] + 0.14 * px, rect[3]];
    out.push(HudQuad::solid(edge, ACCENT));
}

fn draw_dialog(
    menu: &Menu,
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
    let at = [x, panel[1] + 1.0 * px];
    font.text(out, at, 1.1 * px, CHOSEN, "Apply the changes?");
    let at = [x, panel[1] + 2.8 * px];
    let line = "Some settings changed and are not applied yet.";
    font.text(out, at, 0.8 * px, TEXT, line);
    for (k, rect) in buttons.iter().enumerate() {
        let focused = menu.dialog == Some(k);
        if focused {
            out.push(HudQuad::solid(*rect, BAR));
        }
        let border = if focused { ACCENT } else { EDGE };
        frame(*rect, hair.max(0.06 * px), border, out);
        let tx = 0.5 * (rect[0] + rect[2] - m.cells(px, DIALOG[k]));
        let ty = rect[1] + 0.5 * (rect[3] - rect[1] - m.height * px);
        let color = if focused { CHOSEN } else { TEXT };
        font.text(out, [tx, ty], px, color, DIALOG[k]);
    }
}

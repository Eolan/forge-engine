//! The developer console (D-069): the key under Esc (² on AZERTY, ` on QWERTY) opens and closes
//! it over the top of the window, drawn with the overlay's text.
//!
//! A line sets a variable (`r.display_info 2`, or `r.display_info=2`), shows one (its name
//! alone), or runs a command: the console's own ([`crate::settings::BUILTIN`]: `help`, `find`,
//! `list`, `reset`, `version`, `clear`), else the shell's or the demo's. Tab completes a name, then
//! a choice's value; the names that fit show under the line as it is typed. Up and Down walk the
//! history, which is kept between runs; the values set are not (the owner, 2026-10-17).
//!
//! `bind KEY LINE` runs a line when a key is pressed with the console closed (the developer
//! keys left the demos for the console, the owner, 2026-10-17); what it printed shows for a few
//! seconds at the bottom of the window.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use winit::keyboard::KeyCode;

use crate::overlay::{Canvas, Color};
use crate::settings::{self, BUILTIN, Settings, Source};

/// The history lines kept between runs.
const HISTORY_KEPT: usize = 200;
/// The log lines kept.
const LOG_KEPT: usize = 500;
/// The names listed under the line as it is typed.
const SUGGESTIONS: usize = 10;
/// How long a bound key's outcome shows with the console closed, and how many lines.
const NOTICE_TIME: Duration = Duration::from_secs(3);
const NOTICES: usize = 4;
/// The keys the shell keeps: the console's own, Escape, F1 (`r.display_info`).
const RESERVED_KEYS: &[&str] = &["²", "`", "escape", "f1"];

/// What the console asks the shell to do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Run a command the shell or the demo registered.
    Command {
        /// Its name.
        name: String,
        /// Its arguments.
        args: Vec<String>,
    },
    /// Close the console.
    Close,
}

/// The console's state: the line, the history and the log.
pub struct Console {
    open: bool,
    line: String,
    /// The cursor, in characters (the line is ASCII).
    cursor: usize,
    history: Vec<String>,
    /// While walking the history: the entry shown, and the line typed before.
    browsing: Option<usize>,
    draft: String,
    log: Vec<(String, Color)>,
    /// Lines scrolled back from the newest.
    scroll: usize,
    /// Where the history is kept between runs (interactive runs only).
    history_path: Option<PathBuf>,
    /// Tab's walk through the candidates once their common prefix cannot grow.
    cycle: Option<Cycle>,
    /// The keys bound to console lines (`bind`), by name.
    binds: BTreeMap<String, String>,
    /// What the bound keys printed, and when, shown a while with the console closed.
    notices: Vec<(String, Color, Instant)>,
    /// The lines printed since the start.
    printed: usize,
}

/// The candidates Tab steps through, and the one shown.
struct Cycle {
    prefix: String,
    candidates: Vec<String>,
    at: usize,
}

impl Cycle {
    fn line(&self) -> String {
        format!("{}{}", self.prefix, self.candidates[self.at])
    }
}

impl Console {
    /// A closed console whose history is kept in `history_path` (none: not kept).
    pub fn new(history_path: Option<PathBuf>) -> Self {
        let history = history_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| {
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut console = Self {
            open: false,
            line: String::new(),
            cursor: 0,
            history,
            browsing: None,
            draft: String::new(),
            log: Vec::new(),
            scroll: 0,
            history_path,
            cycle: None,
            binds: BTreeMap::new(),
            notices: Vec::new(),
            printed: 0,
        };
        console.print(settings::version(), Color::Dim);
        console.print(
            "Tab completes, Up/Down recall, `help` explains, Esc or the key under Esc closes."
                .to_owned(),
            Color::Dim,
        );
        console
    }

    /// Whether it shows and takes the keys.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens or closes it.
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
        self.scroll = 0;
    }

    /// Adds a line to the log.
    pub fn print(&mut self, text: String, color: Color) {
        for line in text.lines() {
            self.log.push((line.to_owned(), color));
            self.printed += 1;
        }
        if self.log.len() > LOG_KEPT {
            let extra = self.log.len() - LOG_KEPT;
            self.log.drain(..extra);
        }
    }

    /// The line being typed (for tests).
    pub fn line(&self) -> &str {
        &self.line
    }

    /// A key while open: `text` is what it types (winit's `KeyEvent::text`), `ctrl` and `shift`
    /// whether those are held.
    pub fn key(
        &mut self,
        code: KeyCode,
        text: Option<&str>,
        ctrl: bool,
        shift: bool,
        settings: &mut Settings,
    ) -> Option<Request> {
        if matches!(
            code,
            KeyCode::ShiftLeft
                | KeyCode::ShiftRight
                | KeyCode::ControlLeft
                | KeyCode::ControlRight
                | KeyCode::AltLeft
                | KeyCode::AltRight
        ) {
            return None;
        }
        if code != KeyCode::Tab {
            self.cycle = None;
        }
        match code {
            KeyCode::Escape => return Some(Request::Close),
            KeyCode::Enter | KeyCode::NumpadEnter => return self.submit(settings),
            KeyCode::Tab => self.complete(settings, shift),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.line.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.line.len() {
                    self.line.remove(self.cursor);
                }
            }
            KeyCode::ArrowLeft => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::ArrowRight => self.cursor = (self.cursor + 1).min(self.line.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.line.len(),
            KeyCode::ArrowUp => self.recall(true),
            KeyCode::ArrowDown => self.recall(false),
            KeyCode::PageUp => self.scroll = (self.scroll + 10).min(self.log.len()),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::KeyL if ctrl => self.log.clear(),
            KeyCode::KeyU if ctrl => {
                self.line.clear();
                self.cursor = 0;
            }
            _ if !ctrl => {
                // Printable ASCII only: names and values are, and the overlay draws no other.
                for ch in text
                    .unwrap_or("")
                    .chars()
                    .filter(|c| (' '..='~').contains(c))
                {
                    self.line.insert(self.cursor, ch);
                    self.cursor += 1;
                }
            }
            _ => {}
        }
        None
    }

    /// Tab (`back`: Shift+Tab): the line completed as far as the names that fit agree; once
    /// it cannot grow, one candidate after another (Doom 3's and Readline's way).
    fn complete(&mut self, settings: &Settings, back: bool) {
        if let Some(cycle) = &mut self.cycle
            && cycle.line() == self.line
        {
            let n = cycle.candidates.len();
            cycle.at = if back {
                (cycle.at + n - 1) % n
            } else {
                (cycle.at + 1) % n
            };
            self.line = cycle.line();
            self.cursor = self.line.len();
            return;
        }
        let completion = settings.complete(&self.line);
        if completion.line != self.line {
            self.line = completion.line;
            self.cycle = None;
        } else if !completion.candidates.is_empty() {
            let n = completion.candidates.len();
            let cycle = Cycle {
                prefix: completion.prefix,
                candidates: completion.candidates,
                at: if back { n - 1 } else { 0 },
            };
            self.line = cycle.line();
            self.cycle = Some(cycle);
        }
        self.cursor = self.line.len();
    }

    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.browsing, older) {
            (None, true) => {
                self.draft = self.line.clone();
                Some(self.history.len() - 1)
            }
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.browsing = next;
        self.line = match next {
            Some(i) => self.history[i].clone(),
            None => std::mem::take(&mut self.draft),
        };
        self.cursor = self.line.len();
    }

    fn submit(&mut self, settings: &mut Settings) -> Option<Request> {
        let line = std::mem::take(&mut self.line).trim().to_owned();
        self.cursor = 0;
        self.browsing = None;
        self.scroll = 0;
        if line.is_empty() {
            return None;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY_KEPT {
                let extra = self.history.len() - HISTORY_KEPT;
                self.history.drain(..extra);
            }
            self.save_history();
        }
        self.print(format!("> {line}"), Color::Dim);
        self.run(&line, settings)
    }

    fn save_history(&self) {
        let Some(path) = &self.history_path else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, self.history.join("\n") + "\n") {
            tracing::warn!("console history {}: {e}", path.display());
        }
    }

    /// Runs a line: a variable's value set or shown, the console's own commands, else a request.
    pub fn run(&mut self, line: &str, settings: &mut Settings) -> Option<Request> {
        // `name value`, `name=value` and `name = value` alike.
        let (name, rest) = match line.split_once(|c: char| c == '=' || c.is_whitespace()) {
            Some((name, rest)) => (name.trim(), rest.trim().trim_start_matches('=').trim()),
            None => (line.trim(), ""),
        };
        let args: Vec<String> = rest.split_whitespace().map(str::to_owned).collect();
        // Names in any case; `NAME ?` is `help NAME`.
        let name = name.to_ascii_lowercase();
        let name = name.as_str();
        let (name, args) = if rest == "?" {
            ("help", vec![name.to_owned()])
        } else {
            (name, args)
        };
        match name {
            "clear" => self.log.clear(),
            "version" => self.print(settings::version(), Color::White),
            "bind" => self.bind(rest),
            "unbind" => match args.first() {
                Some(key) => {
                    let key = key.to_lowercase();
                    match self.binds.remove(&key) {
                        Some(line) => self.print(format!("{key} freed (was {line})"), Color::Green),
                        None => self.print(format!("{key} is not bound"), Color::Grey),
                    }
                }
                None => self.print("unbind KEY".to_owned(), Color::Red),
            },
            "binds" => {
                if self.binds.is_empty() {
                    self.print(
                        "no key is bound (bind KEY LINE; forge.cfg keeps them)".to_owned(),
                        Color::Grey,
                    );
                }
                let lines: Vec<String> = self
                    .binds
                    .iter()
                    .map(|(key, line)| format!("  {key}  {line}"))
                    .collect();
                for line in lines {
                    self.print(line, Color::Grey);
                }
            }
            "changed" => {
                let lines = settings.changed();
                if lines.is_empty() {
                    self.print("every setting is at its default".to_owned(), Color::Grey);
                }
                for line in lines {
                    self.print(format!("  {line}"), Color::Grey);
                }
            }
            "toggle" => match args.first() {
                Some(n) => {
                    let result = settings.toggle(&n.to_ascii_lowercase(), Source::Console);
                    self.report(result);
                }
                None => self.print("toggle NAME".to_owned(), Color::Red),
            },
            "help" if args.is_empty() => self.help(settings),
            "help" => match settings.describe(&args[0].to_ascii_lowercase()) {
                Some(text) => self.print(text, Color::White),
                None => self.print(
                    format!("nothing is named '{}' (find WORD searches)", args[0]),
                    Color::Red,
                ),
            },
            "find" | "list" => {
                let word = args.first().map_or("", String::as_str);
                let names = if name == "find" {
                    settings.names_containing(word)
                } else {
                    settings.complete(word).candidates
                };
                if names.is_empty() {
                    self.print(format!("nothing under '{word}'"), Color::Red);
                }
                for n in names {
                    let text = settings.describe(&n).unwrap_or(n);
                    self.print(format!("  {text}"), Color::Grey);
                }
            }
            "reset" => match args.first() {
                Some(n) => {
                    let result = settings.reset(&n.to_ascii_lowercase());
                    self.report(result);
                }
                None => self.print("reset NAME".to_owned(), Color::Red),
            },
            _ if settings.info(name).is_some() => {
                if rest.is_empty() {
                    let text = settings.describe(name).unwrap_or_default();
                    self.print(text, Color::White);
                } else {
                    let result = settings.set_text(name, rest, Source::Console);
                    self.report(result);
                }
            }
            _ if settings.command(name).is_some() => {
                return Some(Request::Command {
                    name: name.to_owned(),
                    args,
                });
            }
            _ => self.print(
                format!("nothing is named '{name}' (Tab completes, find WORD searches)"),
                Color::Red,
            ),
        }
        None
    }

    /// `bind KEY [LINE]`: binds KEY to LINE (quotes round it are dropped), or shows its line.
    fn bind(&mut self, rest: &str) {
        let (key, line) = match rest.split_once(char::is_whitespace) {
            Some((key, line)) => (key, line.trim()),
            None => (rest, ""),
        };
        let key = key.trim().to_lowercase();
        let line = line
            .strip_prefix('"')
            .and_then(|l| l.strip_suffix('"'))
            .unwrap_or(line)
            .trim();
        if key.is_empty() {
            self.print("bind KEY [LINE]".to_owned(), Color::Red);
        } else if RESERVED_KEYS.contains(&key.as_str()) {
            self.print(
                format!("{key} is the shell's (the console, Escape and F1 stay)"),
                Color::Red,
            );
        } else if line.is_empty() {
            let text = match self.binds.get(&key) {
                Some(line) => format!("{key}  {line}"),
                None => format!("{key} is not bound"),
            };
            self.print(text, Color::White);
        } else {
            self.print(format!("{key} runs {line}"), Color::Green);
            self.binds.insert(key, line.to_owned());
        }
    }

    /// The line bound to the key named `key` ([`key_name`]), if any.
    pub fn bound(&self, key: &str) -> Option<&str> {
        self.binds.get(key).map(String::as_str)
    }

    /// Runs `line`, its commands split at `;`: what the console asks the shell, in order.
    pub fn run_lines(&mut self, line: &str, settings: &mut Settings) -> Vec<Request> {
        line.split(';')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter_map(|l| self.run(l, settings))
            .collect()
    }

    /// Runs the line bound to `key` with the console closed, if any; what it printed shows a
    /// while ([`Console::layout_notices`]).
    pub fn run_bound(&mut self, key: &str, settings: &mut Settings) -> Option<Vec<Request>> {
        let line = self.binds.get(key)?.clone();
        let from = self.printed;
        let requests = self.run_lines(&line, settings);
        let now = Instant::now();
        // What it printed is at the log's end (which may have dropped its oldest lines).
        let new = (self.printed - from).min(self.log.len());
        let lines: Vec<(String, Color)> = self.log[self.log.len() - new..].to_vec();
        self.notices
            .retain(|(_, _, at)| now.duration_since(*at) < NOTICE_TIME);
        self.notices
            .extend(lines.into_iter().map(|(t, c)| (t, c, now)));
        let excess = self.notices.len().saturating_sub(NOTICES);
        self.notices.drain(..excess);
        Some(requests)
    }

    /// Whether a bound key's outcome is still to show at `now`.
    pub fn has_notices(&self, now: Instant) -> bool {
        self.notices
            .iter()
            .any(|(_, _, at)| now.duration_since(*at) < NOTICE_TIME)
    }

    /// Draws the bound keys' recent outcomes at the bottom left of `canvas` (the console
    /// closed).
    pub fn layout_notices(&self, canvas: &mut Canvas, now: Instant) {
        let shown: Vec<&(String, Color, Instant)> = self
            .notices
            .iter()
            .filter(|(_, _, at)| now.duration_since(*at) < NOTICE_TIME)
            .collect();
        let rows = canvas.rows();
        if shown.is_empty() || rows < shown.len() + 1 {
            return;
        }
        let width = shown.iter().map(|(t, ..)| t.len()).max().unwrap_or(0) + 2;
        let width = width.min(canvas.cols());
        let top = rows - shown.len() - 1;
        canvas.clear(0, top, width, shown.len());
        canvas.panel(0, top, width, shown.len(), Color::PanelDark);
        for (i, (text, color, _)) in shown.iter().enumerate() {
            canvas.text(1, top + i, text, *color);
        }
    }

    /// Prints a command's or a change's outcome.
    pub fn report(&mut self, result: Result<String, String>) {
        match result {
            Ok(text) => self.print(text, Color::Green),
            Err(text) => self.print(text, Color::Red),
        }
    }

    fn help(&mut self, settings: &Settings) {
        let lines = [
            "NAME VALUE sets a setting for this run (NAME=VALUE too); NAME alone shows it.",
            "Settings are dotted by system: r. rendering, app. the shell; Tab after a dot lists them.",
            "forge.cfg (name value lines) and --set NAME=VALUE set them at start.",
        ];
        for line in lines {
            self.print(line.to_owned(), Color::White);
        }
        for (name, usage, help) in BUILTIN {
            self.print(format!("  {name} {usage}  {help}"), Color::Grey);
        }
        for c in settings.commands() {
            self.print(format!("  {} {}  {}", c.name, c.usage, c.help), Color::Grey);
        }
    }

    /// Draws it over the top two fifths of `canvas`, and the names that fit the line under it.
    pub fn layout(&self, canvas: &mut Canvas, settings: &Settings) {
        let cols = canvas.cols();
        let rows = (canvas.rows() * 2 / 5).max(6).min(canvas.rows());
        if cols < 10 || rows < 3 {
            return;
        }
        canvas.clear(0, 0, cols, rows);
        canvas.panel(0, 0, cols, rows, Color::PanelDark);
        canvas.panel(0, 0, cols, 1, Color::Header);
        canvas.text(1, 0, "console", Color::Yellow);
        let version = settings::version();
        canvas.text(
            cols.saturating_sub(version.len() + 1),
            0,
            &version,
            Color::Dim,
        );
        // The log, newest at the bottom, above the line.
        let log_rows = rows - 2;
        let end = self.log.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(log_rows);
        for (i, (text, color)) in self.log[start..end].iter().enumerate() {
            canvas.text(1, 1 + i, text, *color);
        }
        let input = rows - 1;
        canvas.panel(0, input, cols, 1, Color::Panel);
        canvas.text(1, input, ">", Color::Yellow);
        // The line scrolls left to keep the cursor in view.
        let width = cols.saturating_sub(4);
        let skip = (self.cursor + 1).saturating_sub(width);
        canvas.text(3, input, self.line.get(skip..).unwrap_or(""), Color::White);
        canvas.panel(3 + self.cursor - skip, input, 1, 1, Color::Blue);
        // What fits the line, with its value and help; while Tab steps through them, those, the
        // current one marked.
        let (suggestions, current) = match &self.cycle {
            Some(cycle) if cycle.line() == self.line => (cycle.candidates.clone(), Some(cycle.at)),
            _ if self.line.trim().is_empty() => (Vec::new(), None),
            _ => (settings.complete(&self.line).candidates, None),
        };
        let shown = suggestions.len().min(SUGGESTIONS);
        if shown == 0 || rows + shown > canvas.rows() {
            return;
        }
        // The window of the list that holds the current one.
        let first = current.map_or(0, |at| (at + 1).saturating_sub(shown));
        let more = suggestions.len() - shown;
        let height = shown + usize::from(more > 0);
        canvas.clear(0, rows, cols, height);
        canvas.panel(0, rows, cols, height, Color::Panel);
        for (i, name) in suggestions.iter().skip(first).take(shown).enumerate() {
            let text = settings.describe(name).unwrap_or_else(|| name.clone());
            let marked = current == Some(first + i);
            if marked {
                canvas.panel(0, rows + i, cols, 1, Color::Header);
            }
            let color = if marked || (current.is_none() && i == 0) {
                Color::White
            } else {
                Color::Grey
            };
            canvas.text(3, rows + i, &text, color);
        }
        if more > 0 {
            canvas.text(
                3,
                rows + shown,
                &format!("... {more} more (Tab steps through them)"),
                Color::Dim,
            );
        }
    }
}

/// Where the history is kept: `%LOCALAPPDATA%/forge` on Windows, `$XDG_DATA_HOME/forge` or
/// `~/.local/share/forge` elsewhere.
pub fn history_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }?;
    Some(base.join("forge").join("console_history.txt"))
}

/// A key's name for `bind`: what it types in the keyboard's layout, without the modifiers (`t`,
/// `z`, `²`, `&` on AZERTY's 1), or its name when it types no text (`f5`, `tab`, `space`,
/// `pageup`, `up`, `numpad1`).
pub fn key_name(event: &winit::event::KeyEvent) -> Option<String> {
    let winit::keyboard::PhysicalKey::Code(code) = event.physical_key else {
        return None;
    };
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    let key = {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        event.key_without_modifiers()
    };
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let key = event.logical_key.clone();
    let label = match &key {
        winit::keyboard::Key::Character(text) => Some(text.as_str()),
        _ => None,
    };
    Some(name_of(code, label))
}

/// The name of the key at `code` that types `label` ([`key_name`]).
fn name_of(code: KeyCode, label: Option<&str>) -> String {
    use KeyCode as K;
    // The main block's keys that type text are named by it: the layout's letters.
    let types_text = matches!(
        code,
        K::KeyA
            | K::KeyB
            | K::KeyC
            | K::KeyD
            | K::KeyE
            | K::KeyF
            | K::KeyG
            | K::KeyH
            | K::KeyI
            | K::KeyJ
            | K::KeyK
            | K::KeyL
            | K::KeyM
            | K::KeyN
            | K::KeyO
            | K::KeyP
            | K::KeyQ
            | K::KeyR
            | K::KeyS
            | K::KeyT
            | K::KeyU
            | K::KeyV
            | K::KeyW
            | K::KeyX
            | K::KeyY
            | K::KeyZ
            | K::Digit0
            | K::Digit1
            | K::Digit2
            | K::Digit3
            | K::Digit4
            | K::Digit5
            | K::Digit6
            | K::Digit7
            | K::Digit8
            | K::Digit9
            | K::Backquote
            | K::Minus
            | K::Equal
            | K::BracketLeft
            | K::BracketRight
            | K::Backslash
            | K::Semicolon
            | K::Quote
            | K::Comma
            | K::Period
            | K::Slash
            | K::IntlBackslash
            | K::IntlRo
            | K::IntlYen
    );
    if types_text && let Some(label) = label.filter(|l| !l.trim().is_empty()) {
        return label.to_lowercase();
    }
    match code {
        K::ArrowUp => "up".to_owned(),
        K::ArrowDown => "down".to_owned(),
        K::ArrowLeft => "left".to_owned(),
        K::ArrowRight => "right".to_owned(),
        _ => format!("{code:?}").to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Spec;

    fn typed(console: &mut Console, settings: &mut Settings, text: &str) -> Option<Request> {
        for ch in text.chars() {
            console.key(KeyCode::KeyA, Some(&ch.to_string()), false, false, settings);
        }
        console.key(KeyCode::Enter, None, false, false, settings)
    }

    #[test]
    fn a_line_sets_shows_and_resets_a_setting() {
        let mut settings = Settings::new();
        let info = settings.add(Spec::int("r.display_info", 1, 0..=2, "frame times").release());
        let mut console = Console::new(None);
        assert_eq!(typed(&mut console, &mut settings, "r.display_info 2"), None);
        assert_eq!(settings.get(info), 2);
        typed(&mut console, &mut settings, "r.display_info=0");
        assert_eq!(settings.get(info), 0);
        typed(&mut console, &mut settings, "reset r.display_info");
        assert_eq!(settings.get(info), 1);
        // Out of range: refused, the value kept, the reason in red.
        typed(&mut console, &mut settings, "r.display_info 7");
        assert_eq!(settings.get(info), 1);
        assert_eq!(console.log.last().map(|(_, c)| *c), Some(Color::Red));
    }

    #[test]
    fn tab_completes_and_history_recalls() {
        let mut settings = Settings::new();
        settings.add(Spec::int("r.display_info", 1, 0..=2, "frame times"));
        settings.add(Spec::choice(
            "r.aa",
            &["off", "taa", "dlaa"],
            1,
            "anti-aliasing",
        ));
        let mut console = Console::new(None);
        for ch in "r.d".chars() {
            console.key(
                KeyCode::KeyA,
                Some(&ch.to_string()),
                false,
                false,
                &mut settings,
            );
        }
        console.key(KeyCode::Tab, None, false, false, &mut settings);
        assert_eq!(console.line(), "r.display_info ");
        console.key(KeyCode::Digit2, Some("2"), false, false, &mut settings);
        console.key(KeyCode::Enter, None, false, false, &mut settings);
        console.key(KeyCode::ArrowUp, None, false, false, &mut settings);
        assert_eq!(console.line(), "r.display_info 2");
        console.key(KeyCode::ArrowDown, None, false, false, &mut settings);
        assert_eq!(console.line(), "");
        // The key under Esc's own character (² or `) is never typed: the shell takes the key.
        console.key(KeyCode::KeyA, Some("\u{b2}"), false, false, &mut settings);
        assert_eq!(console.line(), "");
    }

    #[test]
    fn commands_go_to_the_shell() {
        let mut settings = Settings::new();
        settings.add_command("app.capture", "[PATH]", "saves the next frame", false);
        let mut console = Console::new(None);
        assert_eq!(
            typed(&mut console, &mut settings, "app.capture shot.png"),
            Some(Request::Command {
                name: "app.capture".to_owned(),
                args: vec!["shot.png".to_owned()],
            })
        );
        assert_eq!(
            console.key(KeyCode::Escape, None, false, false, &mut settings),
            Some(Request::Close)
        );
    }

    #[test]
    fn tab_steps_through_the_candidates_once_the_prefix_cannot_grow() {
        let mut settings = Settings::new();
        settings.add(Spec::bool("r.ao", true, "ambient occlusion"));
        settings.add(Spec::float(
            "r.ao.radius",
            1.5,
            0.1..=8.0,
            "occlusion radius",
        ));
        settings.add(Spec::choice(
            "r.aa",
            &["off", "taa", "dlaa"],
            1,
            "anti-aliasing",
        ));
        let mut console = Console::new(None);
        let mut tab = |console: &mut Console, back: bool| {
            console.key(KeyCode::Tab, None, false, back, &mut settings);
            console.line().to_owned()
        };
        let typing = |console: &mut Console, text: &str| {
            for ch in text.chars() {
                console.key(
                    KeyCode::KeyA,
                    Some(&ch.to_string()),
                    false,
                    false,
                    &mut Settings::new(),
                );
            }
        };
        typing(&mut console, "r.");
        // `r.` grows to `r.a`, then steps through r.aa, r.ao, r.ao.radius; Shift+Tab goes back.
        assert_eq!(tab(&mut console, false), "r.a");
        assert_eq!(tab(&mut console, false), "r.aa");
        assert_eq!(tab(&mut console, false), "r.ao");
        assert_eq!(tab(&mut console, false), "r.ao.radius");
        assert_eq!(tab(&mut console, false), "r.aa");
        assert_eq!(tab(&mut console, true), "r.ao.radius");
        assert_eq!(tab(&mut console, true), "r.ao");
        // A typed key ends the walk; a choice's values after its name.
        console.key(KeyCode::Backspace, None, false, false, &mut Settings::new());
        typing(&mut console, "a ");
        assert_eq!(console.line(), "r.aa ");
        assert_eq!(tab(&mut console, false), "r.aa off");
        assert_eq!(tab(&mut console, false), "r.aa taa");
    }

    #[test]
    fn toggle_changed_and_the_question_mark() {
        let mut settings = Settings::new();
        let ao = settings.add(Spec::bool("r.ao", true, "ambient occlusion"));
        let mut console = Console::new(None);
        typed(&mut console, &mut settings, "toggle R.AO");
        assert!(!settings.get(ao));
        typed(&mut console, &mut settings, "changed");
        assert_eq!(
            console.log.last().map(|(t, _)| t.as_str()),
            Some("  r.ao = 0  (default 1, from console)")
        );
        typed(&mut console, &mut settings, "r.ao ?");
        assert!(
            console
                .log
                .last()
                .is_some_and(|(t, _)| t.starts_with("r.ao = 0"))
        );
    }

    #[test]
    fn keys_bind_to_lines() {
        let mut settings = Settings::new();
        let ao = settings.add(Spec::bool("r.ao", true, "ambient occlusion"));
        settings.add_command("app.capture", "[PATH]", "saves a frame", false);
        let mut console = Console::new(None);
        typed(&mut console, &mut settings, "bind F5 \"toggle r.ao\"");
        assert_eq!(console.bound("f5"), Some("toggle r.ao"));
        // A bound key runs its line, and what it printed shows a while.
        let start = Instant::now();
        assert_eq!(console.run_bound("f5", &mut settings), Some(Vec::new()));
        assert!(!settings.get(ao));
        assert!(console.has_notices(start));
        assert!(!console.has_notices(start + NOTICE_TIME + NOTICE_TIME));
        assert_eq!(console.run_bound("f6", &mut settings), None);
        // `;` between commands; a command goes to the shell.
        typed(
            &mut console,
            &mut settings,
            "bind g toggle r.ao; app.capture shot.png",
        );
        let requests = console.run_bound("g", &mut settings).unwrap();
        assert!(settings.get(ao));
        assert_eq!(
            requests,
            [Request::Command {
                name: "app.capture".to_owned(),
                args: vec!["shot.png".to_owned()],
            }]
        );
        // The shell's keys stay its own; unbind frees one; binds lists them.
        typed(&mut console, &mut settings, "bind f1 toggle r.ao");
        assert_eq!(console.bound("f1"), None);
        typed(&mut console, &mut settings, "unbind f5");
        assert_eq!(console.bound("f5"), None);
        typed(&mut console, &mut settings, "binds");
        assert_eq!(
            console.log.last().map(|(t, _)| t.as_str()),
            Some("  g  toggle r.ao; app.capture shot.png")
        );
    }

    #[test]
    fn keys_are_named_by_what_they_type() {
        // AZERTY: the key at QWERTY's W types z, the one at 1 types &.
        assert_eq!(name_of(KeyCode::KeyW, Some("z")), "z");
        assert_eq!(name_of(KeyCode::Digit1, Some("&")), "&");
        assert_eq!(name_of(KeyCode::KeyT, Some("T")), "t");
        assert_eq!(name_of(KeyCode::F5, None), "f5");
        assert_eq!(name_of(KeyCode::Space, Some(" ")), "space");
        assert_eq!(name_of(KeyCode::Numpad1, Some("1")), "numpad1");
        assert_eq!(name_of(KeyCode::ArrowUp, None), "up");
        assert_eq!(name_of(KeyCode::PageDown, None), "pagedown");
    }
}

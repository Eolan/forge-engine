//! Settings (D-069): console variables that the engine and the demos register under dotted names
//! (`r.display_info`, `r.ao`), typed, with a default, a range and a line of help.
//!
//! A value comes from, in order, its default, `forge.cfg` (a file edited by hand), the command
//! line (`--set NAME=VALUE`, forge-app's `SettingsArgs`) and the console (the key under Esc,
//! forge-app's `console`), whose changes last for the run only (the owner, 2026-10-17). Systems
//! read their variables every frame: a change applies at once, unless the variable is read at
//! start ([`Flags::RESTART`]). It lives here so that every system registers its own: the
//! renderer's shared names are `forge_render::settings`'.
//!
//! A shipped game (the `shipping` feature) keeps only the variables marked [`Flags::RELEASE`],
//! which show information and give no advantage (frame times and statistics, as Star Citizen's
//! `r_displayInfo` does); the others keep their defaults and are neither listed nor set.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::ops::RangeInclusive;
use std::path::Path;

/// Whether this build is a shipped game's (the `shipping` feature): only [`Flags::RELEASE`]
/// variables are visible and settable.
pub const SHIPPING: bool = cfg!(feature = "shipping");

/// What a variable holds.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// On or off: `1`, `0`, `on`, `off`, `true`, `false`.
    Bool,
    /// A whole number in a range.
    Int {
        /// Least value.
        min: i64,
        /// Greatest value.
        max: i64,
    },
    /// A number in a range.
    Float {
        /// Least value.
        min: f64,
        /// Greatest value.
        max: f64,
    },
    /// One of named choices, set by name or by index.
    Choice(&'static [&'static str]),
}

/// A variable's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    /// [`Kind::Bool`].
    Bool(bool),
    /// [`Kind::Int`].
    Int(i64),
    /// [`Kind::Float`].
    Float(f64),
    /// [`Kind::Choice`]: the choice's index.
    Choice(usize),
}

/// Marks on a variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(u8);

impl Flags {
    /// Kept in shipped games: it shows information and gives no advantage.
    pub const RELEASE: Self = Self(1);
    /// Read when its system starts: set it in `forge.cfg` or on the command line, not in the
    /// console.
    pub const RESTART: Self = Self(2);
    /// Shown, never set.
    pub const READ_ONLY: Self = Self(4);

    /// Whether every flag of `other` is set.
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Flags {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Where a variable's value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Its default.
    Default,
    /// `forge.cfg`.
    File,
    /// `--set` on the command line.
    CommandLine,
    /// The console (for the run only).
    Console,
    /// The program: a key bound to it (F1 and `r.display_info`), a demo's own flag.
    Code,
}

impl Source {
    /// Where it came from, in words.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::File => "forge.cfg",
            Self::CommandLine => "command line",
            Self::Console => "console",
            Self::Code => "program",
        }
    }
}

/// A registered variable.
#[derive(Clone, Debug)]
pub struct VarInfo {
    /// Dotted name.
    pub name: String,
    /// One line of help.
    pub help: String,
    /// What it holds.
    pub kind: Kind,
    /// Its default.
    pub default: Value,
    /// Its value.
    pub value: Value,
    /// Its marks.
    pub flags: Flags,
    /// Where its value came from.
    pub source: Source,
    /// The settings' change count when it last changed.
    pub changed: u64,
}

impl VarInfo {
    /// Whether this build lists and sets it.
    pub fn visible(&self) -> bool {
        !SHIPPING || self.flags.contains(Flags::RELEASE)
    }

    /// Its value as text (a choice by name).
    pub fn value_text(&self) -> String {
        format_value(&self.kind, self.value)
    }

    /// What it takes, as text: `0..2`, `on|off`, `off|taa|dlaa`.
    pub fn range_text(&self) -> String {
        match &self.kind {
            Kind::Bool => "0|1".to_owned(),
            Kind::Int { min, max } => format!("{min}..{max}"),
            Kind::Float { min, max } => format!("{min}..{max}"),
            Kind::Choice(choices) => choices.join("|"),
        }
    }
}

fn format_value(kind: &Kind, value: Value) -> String {
    match (kind, value) {
        (_, Value::Bool(b)) => u8::from(b).to_string(),
        (_, Value::Int(i)) => i.to_string(),
        (_, Value::Float(f)) => {
            let text = format!("{f:.4}");
            let text = text.trim_end_matches('0');
            text.strip_suffix('.').unwrap_or(text).to_owned()
        }
        (Kind::Choice(choices), Value::Choice(i)) => {
            choices.get(i).copied().unwrap_or("?").to_owned()
        }
        (_, Value::Choice(i)) => i.to_string(),
    }
}

/// The text of a value for `kind`, checked against its range.
fn parse_value(kind: &Kind, text: &str) -> Result<Value, String> {
    let text = text.trim();
    match kind {
        Kind::Bool => match text.to_ascii_lowercase().as_str() {
            "1" | "on" | "true" | "yes" => Ok(Value::Bool(true)),
            "0" | "off" | "false" | "no" => Ok(Value::Bool(false)),
            _ => Err(format!("'{text}' is not 0 or 1")),
        },
        Kind::Int { min, max } => {
            let v: i64 = text
                .parse()
                .map_err(|_| format!("'{text}' is not a whole number"))?;
            if v < *min || v > *max {
                return Err(format!("{v} is outside {min}..{max}"));
            }
            Ok(Value::Int(v))
        }
        Kind::Float { min, max } => {
            let v: f64 = text
                .parse()
                .map_err(|_| format!("'{text}' is not a number"))?;
            if !v.is_finite() || v < *min || v > *max {
                return Err(format!("{v} is outside {min}..{max}"));
            }
            Ok(Value::Float(v))
        }
        Kind::Choice(choices) => {
            let lower = text.to_ascii_lowercase();
            if let Some(i) = choices.iter().position(|c| *c == lower) {
                return Ok(Value::Choice(i));
            }
            match text.parse::<usize>() {
                Ok(i) if i < choices.len() => Ok(Value::Choice(i)),
                _ => Err(format!("'{text}' is not one of {}", choices.join("|"))),
            }
        }
    }
}

/// A type a variable holds: `bool`, `i64`, `f64`, or `usize` (a choice's index).
pub trait VarType: Copy {
    /// The value's own.
    fn from_value(value: Value) -> Self;
    /// As a value.
    fn into_value(self) -> Value;
}

impl VarType for bool {
    fn from_value(value: Value) -> Self {
        matches!(value, Value::Bool(true))
    }
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl VarType for i64 {
    fn from_value(value: Value) -> Self {
        match value {
            Value::Int(i) => i,
            _ => 0,
        }
    }
    fn into_value(self) -> Value {
        Value::Int(self)
    }
}

impl VarType for f64 {
    fn from_value(value: Value) -> Self {
        match value {
            Value::Float(f) => f,
            _ => 0.0,
        }
    }
    fn into_value(self) -> Value {
        Value::Float(self)
    }
}

impl VarType for usize {
    fn from_value(value: Value) -> Self {
        match value {
            Value::Choice(i) => i,
            _ => 0,
        }
    }
    fn into_value(self) -> Value {
        Value::Choice(self)
    }
}

/// A registered variable's handle, to read it every frame ([`Settings::get`]).
#[derive(Debug)]
pub struct Var<T> {
    index: usize,
    _type: PhantomData<T>,
}

impl<T> Clone for Var<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Var<T> {}

/// A variable to register ([`Settings::add`]).
#[derive(Clone, Debug)]
pub struct Spec<T> {
    name: String,
    help: String,
    kind: Kind,
    default: Value,
    flags: Flags,
    _type: PhantomData<T>,
}

impl<T> Spec<T> {
    fn new(name: &str, kind: Kind, default: Value, help: &str) -> Self {
        Self {
            name: name.to_owned(),
            help: help.to_owned(),
            kind,
            default,
            flags: Flags::default(),
            _type: PhantomData,
        }
    }

    /// Kept in shipped games ([`Flags::RELEASE`]).
    pub fn release(mut self) -> Self {
        self.flags = self.flags | Flags::RELEASE;
        self
    }

    /// Read when its system starts ([`Flags::RESTART`]).
    pub fn restart(mut self) -> Self {
        self.flags = self.flags | Flags::RESTART;
        self
    }

    /// Shown, never set ([`Flags::READ_ONLY`]).
    pub fn read_only(mut self) -> Self {
        self.flags = self.flags | Flags::READ_ONLY;
        self
    }
}

impl Spec<bool> {
    /// An on/off variable.
    pub fn bool(name: &str, default: bool, help: &str) -> Self {
        Self::new(name, Kind::Bool, Value::Bool(default), help)
    }
}

impl Spec<i64> {
    /// A whole number in `range`.
    pub fn int(name: &str, default: i64, range: RangeInclusive<i64>, help: &str) -> Self {
        let (min, max) = range.into_inner();
        Self::new(
            name,
            Kind::Int { min, max },
            Value::Int(default.clamp(min, max)),
            help,
        )
    }
}

impl Spec<f64> {
    /// A number in `range`.
    pub fn float(name: &str, default: f64, range: RangeInclusive<f64>, help: &str) -> Self {
        let (min, max) = range.into_inner();
        Self::new(
            name,
            Kind::Float { min, max },
            Value::Float(default.clamp(min, max)),
            help,
        )
    }
}

impl Spec<usize> {
    /// One of `choices` (lower case), the `default`-th first.
    pub fn choice(
        name: &str,
        choices: &'static [&'static str],
        default: usize,
        help: &str,
    ) -> Self {
        Self::new(
            name,
            Kind::Choice(choices),
            Value::Choice(default.min(choices.len().saturating_sub(1))),
            help,
        )
    }
}

/// A console command ([`Settings::add_command`]): the console runs its own (`help`, `find`,
/// `list`, `reset`, `version`, `clear`) and hands the others to the shell and the demo.
#[derive(Clone, Debug)]
pub struct CommandInfo {
    /// Dotted name.
    pub name: String,
    /// Its arguments, as the help shows them (`[PATH]`).
    pub usage: String,
    /// One line of help.
    pub help: String,
    /// Kept in shipped games.
    pub release: bool,
}

impl CommandInfo {
    /// Whether this build lists and runs it.
    pub fn visible(&self) -> bool {
        !SHIPPING || self.release
    }
}

/// An override waiting for its variable to be registered.
#[derive(Clone, Debug)]
struct Pending {
    name: String,
    value: String,
    source: Source,
}

/// The registry of variables and commands, owned by [`crate::Context`].
#[derive(Debug, Default)]
pub struct Settings {
    vars: Vec<VarInfo>,
    by_name: BTreeMap<String, usize>,
    commands: Vec<CommandInfo>,
    pending: Vec<Pending>,
    changes: u64,
    /// `forge.cfg`'s console lines (`bind`, `unbind`), run once the demo is ready.
    startup: Vec<String>,
}

/// The completion of a console line ([`Settings::complete`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Completion {
    /// The line completed as far as every candidate agrees.
    pub line: String,
    /// The line before the word completed (`r.aa ` before a choice): a candidate after it is
    /// a whole line, as Tab cycles through them.
    pub prefix: String,
    /// What it could become: names, or a variable's choices.
    pub candidates: Vec<String>,
}

/// A name's match: a variable, a registered command or one of the console's own.
enum Entry<'a> {
    Var(&'a VarInfo),
    Command(&'a CommandInfo),
    Builtin(&'static str, &'static str, &'static str),
}

/// `name USAGE  help`.
fn usage_line(name: &str, usage: &str, help: &str) -> String {
    if usage.is_empty() {
        format!("{name}  {help}")
    } else {
        format!("{name} {usage}  {help}")
    }
}

impl Settings {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a variable and returns its handle. An override already given for its name
    /// (`forge.cfg`, `--set`) applies now. Registering a name again returns the first
    /// registration's handle, its value kept (a demo made again).
    ///
    /// # Panics
    /// When the name is taken by a variable of another kind, or is not a dotted lower-case
    /// name (`r.display_info`).
    pub fn add<T: VarType>(&mut self, spec: Spec<T>) -> Var<T> {
        assert!(
            valid_name(&spec.name),
            "a setting's name is dotted lower case: '{}'",
            spec.name
        );
        if let Some(&index) = self.by_name.get(&spec.name) {
            assert_eq!(
                self.vars[index].kind, spec.kind,
                "'{}' registered with another kind",
                spec.name
            );
            return Var {
                index,
                _type: PhantomData,
            };
        }
        let index = self.vars.len();
        self.vars.push(VarInfo {
            name: spec.name.clone(),
            help: spec.help,
            kind: spec.kind,
            default: spec.default,
            value: spec.default,
            flags: spec.flags,
            source: Source::Default,
            changed: 0,
        });
        self.by_name.insert(spec.name.clone(), index);
        let (matching, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.name == spec.name);
        self.pending = rest;
        for p in matching {
            if let Err(e) = self.set_from(index, &p.value, p.source) {
                tracing::warn!("{} ({}): {e}", p.name, p.source.describe());
            }
        }
        Var {
            index,
            _type: PhantomData,
        }
    }

    /// Registers `spec` for a program whose flags already set its state (a demo's `--no-ao`):
    /// where they made the value `now` other than the shared `default`, it is set so, as from
    /// the command line. A value from `forge.cfg` or `--set` stays otherwise, and the program
    /// applies it as a change.
    pub fn add_from<T: VarType + PartialEq>(
        &mut self,
        spec: Spec<T>,
        default: T,
        now: T,
    ) -> Var<T> {
        let var = self.add(spec);
        if now != default {
            self.set_by(var, now, Source::CommandLine);
        }
        var
    }

    /// [`Settings::add_from`] for a value the program keeps in `f32`, compared there: a default
    /// written in decimal (0.04) is the flag's own.
    pub fn add_from_f32(&mut self, spec: Spec<f64>, default: f64, now: f32) -> Var<f64> {
        let var = self.add(spec);
        if default as f32 != now {
            self.set_by(var, f64::from(now), Source::CommandLine);
        }
        var
    }

    /// Registers a console command the shell or the demo runs.
    pub fn add_command(&mut self, name: &str, usage: &str, help: &str, release: bool) {
        assert!(
            valid_name(name),
            "a command's name is dotted lower case: '{name}'"
        );
        if self.commands.iter().any(|c| c.name == name) {
            return;
        }
        self.commands.push(CommandInfo {
            name: name.to_owned(),
            usage: usage.to_owned(),
            help: help.to_owned(),
            release,
        });
        self.commands.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Its value.
    pub fn get<T: VarType>(&self, var: Var<T>) -> T {
        T::from_value(self.vars[var.index].value)
    }

    /// Sets it from the program ([`Source::Code`]: a key bound to it), clamped to its range.
    pub fn set<T: VarType>(&mut self, var: Var<T>, value: T) {
        let info = &self.vars[var.index];
        let value = clamp(&info.kind, value.into_value());
        self.store(var.index, value, Source::Code);
    }

    /// Sets it as `source` gave it (a demo's flag, [`Source::CommandLine`]), clamped to its range.
    pub fn set_by<T: VarType>(&mut self, var: Var<T>, value: T, source: Source) {
        let info = &self.vars[var.index];
        let value = clamp(&info.kind, value.into_value());
        self.store(var.index, value, source);
    }

    /// Where its value came from.
    pub fn source<T>(&self, var: Var<T>) -> Source {
        self.vars[var.index].source
    }

    /// Whether it changed since the change count `seen` ([`Settings::changes`]).
    pub fn changed_since<T>(&self, var: Var<T>, seen: u64) -> bool {
        self.vars[var.index].changed > seen
    }

    /// How many changes were made so far.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// The variable named `name`, if this build shows it.
    pub fn info(&self, name: &str) -> Option<&VarInfo> {
        self.by_name
            .get(name)
            .map(|&i| &self.vars[i])
            .filter(|v| v.visible())
    }

    /// The variables this build shows, by name.
    pub fn vars(&self) -> impl Iterator<Item = &VarInfo> {
        self.by_name
            .values()
            .map(|&i| &self.vars[i])
            .filter(|v| v.visible())
    }

    /// The commands this build shows, by name.
    pub fn commands(&self) -> impl Iterator<Item = &CommandInfo> {
        self.commands.iter().filter(|c| c.visible())
    }

    /// The command named `name`, if this build shows it.
    pub fn command(&self, name: &str) -> Option<&CommandInfo> {
        self.commands().find(|c| c.name == name)
    }

    /// Sets `name` (any case) from text, from `source`: now if it is registered, else when it
    /// is. Returns what changed, as the console prints it.
    pub fn set_text(&mut self, name: &str, value: &str, source: Source) -> Result<String, String> {
        let name = name.to_ascii_lowercase();
        let name = name.as_str();
        match self.by_name.get(name).copied() {
            Some(index) => self.set_from(index, value, source),
            None if source == Source::Console => Err(format!("no setting '{name}'")),
            None => {
                self.pending.push(Pending {
                    name: name.to_owned(),
                    value: value.to_owned(),
                    source,
                });
                Ok(format!("{name} waits to be registered"))
            }
        }
    }

    /// Puts `name` back to its default.
    pub fn reset(&mut self, name: &str) -> Result<String, String> {
        let index = self.visible_index(name)?;
        if self.vars[index].flags.contains(Flags::READ_ONLY) {
            return Err(format!("{name} is read-only"));
        }
        let default = self.vars[index].default;
        self.store(index, default, Source::Default);
        Ok(format!(
            "{name} = {} (default)",
            self.vars[index].value_text()
        ))
    }

    /// Flips an on/off setting, or steps a choice or a whole number to its next value, back to
    /// the first after the last (`toggle r.display_info`: 0, 1, 2, 0).
    pub fn toggle(&mut self, name: &str, source: Source) -> Result<String, String> {
        let index = self.visible_index(name)?;
        let info = &self.vars[index];
        let next = match (&info.kind, info.value) {
            (_, Value::Bool(b)) => Value::Bool(!b),
            (Kind::Choice(choices), Value::Choice(i)) => Value::Choice((i + 1) % choices.len()),
            (Kind::Int { min, max }, Value::Int(i)) if max - min < 64 => {
                Value::Int(if i >= *max { *min } else { i + 1 })
            }
            _ => {
                return Err(format!(
                    "{name} takes a value: {name} {}",
                    info.range_text()
                ));
            }
        };
        let text = format_value(&info.kind, next);
        self.set_from(index, &text, source)
    }

    /// [`Settings::toggle`] from the program, by its handle: a key bound to it.
    pub fn toggle_var<T>(&mut self, var: Var<T>) -> Result<String, String> {
        let name = self.vars[var.index].name.clone();
        self.toggle(&name, Source::Code)
    }

    /// The settings not at their default: name, value and where the value came from.
    pub fn changed(&self) -> Vec<String> {
        self.vars()
            .filter(|v| v.value != v.default)
            .map(|v| {
                format!(
                    "{} = {}  (default {}, from {})",
                    v.name,
                    v.value_text(),
                    format_value(&v.kind, v.default),
                    v.source.describe()
                )
            })
            .collect()
    }

    /// The overrides whose variable was never registered: misspelled names, or a demo that has
    /// none of that name, and where they were given.
    pub fn unmatched(&self) -> impl Iterator<Item = (&str, Source)> {
        self.pending.iter().map(|p| (p.name.as_str(), p.source))
    }

    /// Reads `forge.cfg`'s overrides: one `name value` (or `name = value`) a line, `#` or `//`
    /// starting a comment. A line starting with one of [`STARTUP_COMMANDS`] (`bind f5 toggle
    /// r.aa`) is kept for the console to run once the demo is ready ([`Settings::take_startup`]).
    /// A missing file is no error.
    pub fn load_file(&mut self, path: &Path) -> Result<usize, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let mut count = 0;
        for (number, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("");
            let line = line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let first = line.split_whitespace().next().unwrap_or("");
            if STARTUP_COMMANDS.contains(&first.to_ascii_lowercase().as_str()) {
                self.startup.push(line.to_owned());
                count += 1;
                continue;
            }
            let Some((name, value)) = split_assignment(line) else {
                return Err(format!(
                    "{}:{}: '{line}' is not 'name value'",
                    path.display(),
                    number + 1
                ));
            };
            if let Err(e) = self.set_text(name, value, Source::File) {
                tracing::warn!("{}:{}: {e}", path.display(), number + 1);
            }
            count += 1;
        }
        Ok(count)
    }

    /// The console lines `forge.cfg` gave ([`Settings::load_file`]), in their order, once.
    pub fn take_startup(&mut self) -> Vec<String> {
        std::mem::take(&mut self.startup)
    }

    /// Completes a console line: a name (a variable's or a command's) by its prefix, then a
    /// choice variable's value, or the name after `help`, `reset` and `list`.
    pub fn complete(&self, line: &str) -> Completion {
        let trimmed = line.trim_start();
        let (head, tail) = match trimmed.split_once(' ') {
            Some((head, tail)) => (head.to_ascii_lowercase(), Some(tail.trim_start())),
            None => (trimmed.to_ascii_lowercase(), None),
        };
        let head = head.as_str();
        match tail {
            None => {
                let names = self.names_with_prefix(head);
                let common = common_prefix(&names);
                let line = if names.len() == 1 {
                    format!("{} ", names[0])
                } else if common.len() > head.len() {
                    common.clone()
                } else {
                    trimmed.to_owned()
                };
                let candidates = if names.is_empty() {
                    // Nothing starts so: the names with a part starting so (`probe` finds
                    // `gi.probe_spacing`), else holding it anywhere.
                    let parts = self.names_with_part(head);
                    if parts.is_empty() {
                        self.names_containing(head)
                    } else {
                        parts
                    }
                } else {
                    names
                };
                Completion {
                    line,
                    prefix: String::new(),
                    candidates,
                }
            }
            Some(tail) => {
                if matches!(head, "help" | "reset" | "list" | "toggle") {
                    let tail = tail.to_ascii_lowercase();
                    let names = self.names_with_prefix(&tail);
                    let common = common_prefix(&names);
                    let completed = if names.len() == 1 {
                        names[0].clone()
                    } else if common.len() > tail.len() {
                        common
                    } else {
                        tail
                    };
                    return Completion {
                        line: format!("{head} {completed}"),
                        prefix: format!("{head} "),
                        candidates: names,
                    };
                }
                let Some(info) = self.info(head) else {
                    return Completion {
                        line: line.to_owned(),
                        prefix: String::new(),
                        candidates: Vec::new(),
                    };
                };
                let values: Vec<String> = match &info.kind {
                    Kind::Choice(choices) => choices.iter().map(|c| (*c).to_owned()).collect(),
                    Kind::Bool => vec!["0".to_owned(), "1".to_owned()],
                    _ => Vec::new(),
                };
                let lower = tail.to_ascii_lowercase();
                let matching: Vec<String> = values
                    .into_iter()
                    .filter(|v| v.starts_with(&lower))
                    .collect();
                let completed = if matching.len() == 1 {
                    matching[0].clone()
                } else {
                    let common = common_prefix(&matching);
                    if common.len() > tail.len() {
                        common
                    } else {
                        tail.to_owned()
                    }
                };
                Completion {
                    line: format!("{head} {completed}"),
                    prefix: format!("{head} "),
                    candidates: matching,
                }
            }
        }
    }

    /// One line describing a name: `r.ao = 1 (0|1, default 1): …` or a command's usage.
    pub fn describe(&self, name: &str) -> Option<String> {
        match self.entry(name)? {
            Entry::Var(v) => {
                let mut text = format!("{} = {}  [{}", v.name, v.value_text(), v.range_text());
                if v.value != v.default {
                    text += &format!(", default {}", format_value(&v.kind, v.default));
                }
                if v.source != Source::Default {
                    text += &format!(", from {}", v.source.describe());
                }
                if v.flags.contains(Flags::RESTART) {
                    text += ", at start";
                }
                if v.flags.contains(Flags::READ_ONLY) {
                    text += ", read-only";
                }
                Some(format!("{text}]  {}", v.help))
            }
            Entry::Command(c) => Some(usage_line(&c.name, &c.usage, &c.help)),
            Entry::Builtin(name, usage, help) => Some(usage_line(name, usage, help)),
        }
    }

    fn entry(&self, name: &str) -> Option<Entry<'_>> {
        if let Some(v) = self.info(name) {
            return Some(Entry::Var(v));
        }
        if let Some(c) = self.command(name) {
            return Some(Entry::Command(c));
        }
        BUILTIN
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|&(n, usage, help)| Entry::Builtin(n, usage, help))
    }

    fn names(&self) -> impl Iterator<Item = &str> {
        let builtin = BUILTIN.iter().map(|(name, _, _)| *name);
        self.vars()
            .map(|v| v.name.as_str())
            .chain(self.commands().map(|c| c.name.as_str()))
            .chain(builtin)
    }

    fn names_with_prefix(&self, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .names()
            .filter(|n| n.starts_with(prefix))
            .map(str::to_owned)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The names with a dotted part starting with `word`.
    fn names_with_part(&self, word: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .names()
            .filter(|n| !word.is_empty() && n.split('.').any(|part| part.starts_with(word)))
            .map(str::to_owned)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The names holding `word` anywhere (`find`).
    pub fn names_containing(&self, word: &str) -> Vec<String> {
        let word = word.to_ascii_lowercase();
        let mut names: Vec<String> = self
            .names()
            .filter(|n| n.contains(word.as_str()))
            .map(str::to_owned)
            .chain(
                self.vars()
                    .filter(|v| v.help.to_ascii_lowercase().contains(word.as_str()))
                    .map(|v| v.name.clone()),
            )
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn visible_index(&self, name: &str) -> Result<usize, String> {
        match self.by_name.get(name) {
            Some(&index) if self.vars[index].visible() => Ok(index),
            _ => Err(format!("no setting '{name}'")),
        }
    }

    fn set_from(&mut self, index: usize, text: &str, source: Source) -> Result<String, String> {
        let info = &self.vars[index];
        if !info.visible() {
            return Err(format!("no setting '{}'", info.name));
        }
        if info.flags.contains(Flags::READ_ONLY) {
            return Err(format!("{} is read-only", info.name));
        }
        if source == Source::Console && info.flags.contains(Flags::RESTART) {
            return Err(format!(
                "{} is read at start: set it in forge.cfg or with --set",
                info.name
            ));
        }
        let value = parse_value(&info.kind, text)?;
        let before = info.value;
        let was = (before != value).then(|| format_value(&info.kind, before));
        self.store(index, value, source);
        let info = &self.vars[index];
        Ok(match was {
            Some(was) => format!("{} = {} (was {was})", info.name, info.value_text()),
            None => format!("{} = {}", info.name, info.value_text()),
        })
    }

    fn store(&mut self, index: usize, value: Value, source: Source) {
        let info = &mut self.vars[index];
        info.source = source;
        if info.value != value {
            info.value = value;
            self.changes += 1;
            info.changed = self.changes;
        }
    }
}

/// The console's own commands: name, usage, help.
pub const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "bind",
        "KEY [LINE]",
        "Runs LINE when KEY is pressed with the console closed (`bind f5 toggle r.aa`; `;` \
         between several); KEY alone shows its line. KEY is what the key types (t, z, ²) or \
         its name (f5, tab, space, pageup, numpad1). For the run; in forge.cfg, every run.",
    ),
    ("binds", "", "Lists the keys bound to console lines."),
    (
        "changed",
        "",
        "Lists the settings not at their default, and where their values came from.",
    ),
    ("clear", "", "Empties the console."),
    (
        "find",
        "WORD",
        "Lists the settings and commands whose name or help holds WORD.",
    ),
    (
        "help",
        "[NAME]",
        "Describes a setting or a command; alone, how the console works.",
    ),
    (
        "list",
        "[PREFIX]",
        "Lists the settings and commands, or those under PREFIX (r.).",
    ),
    ("reset", "NAME", "Puts a setting back to its default."),
    (
        "toggle",
        "NAME",
        "Flips an on/off setting, or steps a choice or a small number to its next value.",
    ),
    ("unbind", "KEY", "Frees a key bound to a console line."),
    ("version", "", "The build's version."),
];

/// The console's commands `forge.cfg` may hold beside its settings: run once the demo is ready.
pub const STARTUP_COMMANDS: &[&str] = &["bind", "unbind"];

/// The build's version: the crate's, and the commit when the build was given one
/// (`FORGE_BUILD_COMMIT` at compile time, as a game's release build sets it).
pub fn version() -> String {
    let commit = option_env!("FORGE_BUILD_COMMIT").unwrap_or("dev build");
    let kind = if SHIPPING { "release" } else { "development" };
    format!("forge {} ({commit}, {kind})", env!("CARGO_PKG_VERSION"))
}

fn clamp(kind: &Kind, value: Value) -> Value {
    match (kind, value) {
        (Kind::Int { min, max }, Value::Int(i)) => Value::Int(i.clamp(*min, *max)),
        (Kind::Float { min, max }, Value::Float(f)) => Value::Float(f.clamp(*min, *max)),
        (Kind::Choice(choices), Value::Choice(i)) => {
            Value::Choice(i.min(choices.len().saturating_sub(1)))
        }
        _ => value,
    }
}

/// `name value`, `name=value` or `name = value`.
pub fn split_assignment(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    let (name, value) = match text.split_once('=') {
        Some((name, value)) => (name.trim(), value.trim()),
        None => {
            let (name, value) = text.split_once(char::is_whitespace)?;
            (name.trim(), value.trim())
        }
    };
    (!name.is_empty() && !value.is_empty()).then_some((name, value))
}

/// Dotted lower case: letters, digits and underscores between the dots.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

fn common_prefix(names: &[String]) -> String {
    let Some(first) = names.first() else {
        return String::new();
    };
    let mut len = first.len();
    for name in &names[1..] {
        len = len.min(
            first
                .bytes()
                .zip(name.bytes())
                .take_while(|(a, b)| a == b)
                .count(),
        );
    }
    first[..len].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> (Settings, Var<i64>, Var<bool>, Var<usize>, Var<f64>) {
        let mut s = Settings::new();
        let info =
            s.add(Spec::int("r.display_info", 1, 0..=2, "frame times and statistics").release());
        let ao = s.add(Spec::bool("r.ao", true, "ambient occlusion"));
        let aa = s.add(Spec::choice(
            "r.aa",
            &["off", "taa", "dlaa"],
            1,
            "anti-aliasing",
        ));
        let radius = s.add(Spec::float(
            "r.ao.radius",
            1.5,
            0.1..=8.0,
            "occlusion radius, metres",
        ));
        (s, info, ao, aa, radius)
    }

    #[test]
    fn values_parse_by_kind_and_stay_in_range() {
        let (mut s, info, ao, aa, radius) = registry();
        assert_eq!(
            s.set_text("r.display_info", "2", Source::Console),
            Ok("r.display_info = 2 (was 1)".to_owned())
        );
        assert_eq!(s.get(info), 2);
        assert!(s.set_text("r.display_info", "3", Source::Console).is_err());
        assert_eq!(s.get(info), 2);
        s.set_text("r.ao", "off", Source::Console).unwrap();
        assert!(!s.get(ao));
        s.set_text("r.aa", "DLAA", Source::Console).unwrap();
        assert_eq!(s.get(aa), 2);
        s.set_text("r.aa", "0", Source::Console).unwrap();
        assert_eq!(s.get(aa), 0);
        assert!(s.set_text("r.aa", "fxaa", Source::Console).is_err());
        s.set_text("r.ao.radius", "2.25", Source::Console).unwrap();
        assert_eq!(s.get(radius), 2.25);
        assert!(s.set_text("r.ao.radius", "nan", Source::Console).is_err());
        s.set(radius, 100.0);
        assert_eq!(s.get(radius), 8.0);
        assert!(s.set_text("r.nothing", "1", Source::Console).is_err());
        assert_eq!(
            s.reset("r.ao.radius"),
            Ok("r.ao.radius = 1.5 (default)".to_owned())
        );
    }

    #[test]
    fn overrides_wait_for_their_variable_and_unknown_ones_are_reported() {
        let mut s = Settings::new();
        s.set_text("r.ao", "0", Source::File).unwrap();
        s.set_text("r.ao", "1", Source::CommandLine).unwrap();
        s.set_text("r.typo", "1", Source::CommandLine).unwrap();
        let ao = s.add(Spec::bool("r.ao", false, "ambient occlusion"));
        // The command line comes after the file, and wins.
        assert!(s.get(ao));
        assert_eq!(s.info("r.ao").map(|v| v.source), Some(Source::CommandLine));
        assert_eq!(
            s.unmatched().collect::<Vec<_>>(),
            [("r.typo", Source::CommandLine)]
        );
    }

    #[test]
    fn restart_and_read_only_variables_refuse_the_console() {
        let mut s = Settings::new();
        let fallback = s.add(Spec::bool("gpu.force_fallback", false, "no mesh shaders").restart());
        s.add(Spec::int("app.frames", 0, 0..=i64::MAX, "frames drawn").read_only());
        assert!(
            s.set_text("gpu.force_fallback", "1", Source::Console)
                .is_err()
        );
        s.set_text("gpu.force_fallback", "1", Source::CommandLine)
            .unwrap();
        assert!(s.get(fallback));
        assert!(s.set_text("app.frames", "3", Source::CommandLine).is_err());
    }

    #[test]
    fn completion_finds_names_by_prefix_then_values() {
        let (mut s, ..) = registry();
        s.add_command("app.capture", "[PATH]", "saves the next frame", false);
        // `r.` holds four names: the line stays, every one is listed.
        let c = s.complete("r.");
        assert_eq!(c.line, "r.");
        assert_eq!(
            c.candidates,
            ["r.aa", "r.ao", "r.ao.radius", "r.display_info"]
        );
        // `r.d`: one name, completed with a space for its value.
        assert_eq!(s.complete("r.d").line, "r.display_info ");
        // `r.ao`: the common part is the name itself, two candidates.
        assert_eq!(s.complete("r.ao").candidates, ["r.ao", "r.ao.radius"]);
        assert_eq!(s.complete("app.c").line, "app.capture ");
        // A choice's values after its name.
        let c = s.complete("r.aa d");
        assert_eq!(c.line, "r.aa dlaa");
        assert_eq!(s.complete("r.aa ").candidates, ["off", "taa", "dlaa"]);
        // Names after `help`, and the console's own commands.
        assert_eq!(s.complete("help r.di").line, "help r.display_info");
        assert_eq!(s.complete("ver").line, "version ");
        // No prefix matches: the names with a part starting so, then those holding it.
        assert_eq!(s.complete("radius").candidates, ["r.ao.radius"]);
        assert_eq!(s.complete("display").candidates, ["r.display_info"]);
        assert_eq!(s.complete("adi").candidates, ["r.ao.radius"]);
        // Any case; the prefix before a value, for cycling.
        assert_eq!(s.complete("R.D").line, "r.display_info ");
        assert_eq!(s.complete("r.aa ").prefix, "r.aa ");
    }

    #[test]
    fn toggle_steps_and_changed_lists_what_moved() {
        let (mut s, info, ao, aa, _) = registry();
        assert_eq!(
            s.toggle("r.ao", Source::Console),
            Ok("r.ao = 0 (was 1)".to_owned())
        );
        assert!(!s.get(ao));
        s.toggle("r.aa", Source::Console).unwrap();
        assert_eq!(s.get(aa), 2);
        s.toggle("r.aa", Source::Console).unwrap();
        assert_eq!(s.get(aa), 0);
        s.toggle("r.display_info", Source::Console).unwrap();
        s.toggle("r.display_info", Source::Console).unwrap();
        assert_eq!(s.get(info), 0);
        assert!(s.toggle("r.ao.radius", Source::Console).is_err());
        assert_eq!(
            s.changed(),
            [
                "r.aa = off  (default taa, from console)",
                "r.ao = 0  (default 1, from console)",
                "r.display_info = 0  (default 1, from console)",
            ]
        );
    }

    #[test]
    fn the_file_takes_comments_and_both_forms() {
        let dir = std::env::temp_dir().join(format!("forge-cvars-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("forge.cfg");
        std::fs::write(
            &path,
            "# the owner's\nr.display_info 2\nbind f5 toggle r.aa\nr.aa = dlaa // the look\n\
             Unbind t\n",
        )
        .unwrap();
        let (mut s, info, _, aa, _) = registry();
        assert_eq!(s.load_file(&path), Ok(4));
        assert_eq!(s.get(info), 2);
        assert_eq!(s.get(aa), 2);
        // The console's lines wait for the console, in their order, once.
        assert_eq!(s.take_startup(), ["bind f5 toggle r.aa", "Unbind t"]);
        assert!(s.take_startup().is_empty());
        assert_eq!(s.load_file(&dir.join("missing.cfg")), Ok(0));
        std::fs::write(&path, "r.display_info\n").unwrap();
        assert!(s.load_file(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_are_dotted_lower_case() {
        assert!(valid_name("r.display_info"));
        assert!(valid_name("gi.probes.rays"));
        assert!(!valid_name("r_displayInfo"));
        assert!(!valid_name("r..ao"));
        assert!(!valid_name("R.ao"));
        assert!(!valid_name(""));
    }

    #[test]
    fn descriptions_show_the_value_its_range_and_where_it_came_from() {
        let (mut s, ..) = registry();
        s.set_text("r.display_info", "2", Source::Console).unwrap();
        assert_eq!(
            s.describe("r.display_info").as_deref(),
            Some("r.display_info = 2  [0..2, default 1, from console]  frame times and statistics")
        );
        assert_eq!(
            s.describe("r.aa").as_deref(),
            Some("r.aa = taa  [off|taa|dlaa]  anti-aliasing")
        );
    }
}

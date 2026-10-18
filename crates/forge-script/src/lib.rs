//! Scripts in Luau (D-072, #292): a sandboxed host for chunks of Luau, and of Lua 5.1 that Luau
//! reads, through `mlua`.
//!
//! This first step evaluates data. A caller runs a chunk from a string and takes back its result,
//! or the globals it set, as plain Rust values ([`Data`]): a record of data tables written as
//! code, the rules a hand-made map's files build in loops.
//! - **The sandbox:** Luau's (`luaL_sandbox`). The standard libraries and their metatables are
//!   read-only, a chunk's globals go to an environment of its own, and there is no `io`, `os`,
//!   `debug`, `package`, `load`, `loadstring`, `dofile`, `loadfile`, `getfenv` or `setfenv`.
//!   `require` reads only from one folder the caller names ([`Limits::require_root`]).
//! - **The caps:** a budget of Luau's interrupts (it interrupts at calls and loops' back edges), a
//!   wall-clock deadline and a memory cap. Over any, the chunk ends with an error, and the host
//!   goes on.
//! - **Stub globals:** values and Rust functions the caller injects, so code written against
//!   another engine's interface can partly run ([`Script::set`], [`Script::function`]).
//!
//! Not here (D-072's answers): scripts in the game's tick, mods, missions calling Luau. Scripts
//! never run inside the predicted step.

#![forbid(unsafe_code)]

use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::{Lua, LuaOptions, StdLib, Value, VmState};

/// A value a chunk gave back, as plain data.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    /// `nil`.
    Nil,
    /// `true` or `false`.
    Bool(bool),
    /// A number with no fraction that fits 64 bits.
    Integer(i64),
    /// Any other number.
    Number(f64),
    /// A string (lossy where it is not UTF-8).
    String(String),
    /// A table whose keys are exactly 1 to n: its values in order.
    List(Vec<Data>),
    /// Any other table: its entries by key.
    Map(BTreeMap<Key, Data>),
    /// What has no data form: a function, a coroutine, a userdata; its type's name.
    Opaque(&'static str),
}

/// A table's key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// An integer key.
    Integer(i64),
    /// A string key.
    String(String),
    /// Any other key, as its `tostring`-like text (a fractional number, a boolean).
    Other(String),
}

/// What can go wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The chunk does not compile: its name and line are in the message.
    #[error("{0}")]
    Syntax(String),
    /// The chunk failed while running: its name and line are in the message.
    #[error("{0}")]
    Runtime(String),
    /// The chunk ran past its budget of interrupts or its deadline.
    #[error("{chunk}: over its budget ({what})")]
    Budget {
        /// The chunk's name.
        chunk: String,
        /// Which budget.
        what: &'static str,
    },
    /// The chunk asked for more memory than its cap.
    #[error("{chunk}: over its memory cap of {cap} bytes")]
    Memory {
        /// The chunk's name.
        chunk: String,
        /// The cap.
        cap: usize,
    },
    /// A value could not become [`Data`] (a table nested too deep).
    #[error("{0}")]
    Data(String),
    /// The host could not be set up.
    #[error("the script host: {0}")]
    Host(String),
}

/// The caps a [`Script`] runs its chunks under.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    /// Luau's interrupts a chunk may take (one at each call and each loop's back edge, about):
    /// a budget of work, the same from run to run.
    pub interrupts: u64,
    /// The wall-clock time a chunk may take.
    pub deadline: Duration,
    /// The memory the whole state may hold, bytes.
    pub memory: usize,
    /// The folder `require` reads modules from (`a.b` and `a/b` read `a/b.luau` or `a/b.lua`);
    /// None: no `require`.
    pub require_root: Option<PathBuf>,
    /// Whether `load` and `loadstring` compile new code (in the sandbox, under the same caps).
    pub allow_load: bool,
}

impl Default for Limits {
    /// Ten million interrupts, ten seconds, 256 MiB, no `require`, no `load`.
    fn default() -> Self {
        Self {
            interrupts: 10_000_000,
            deadline: Duration::from_secs(10),
            memory: 256 << 20,
            require_root: None,
            allow_load: false,
        }
    }
}

/// The deepest a table nests in [`Data`].
const DEEPEST: usize = 64;

/// A sandboxed Luau state that runs chunks under [`Limits`].
pub struct Script {
    lua: Lua,
    limits: Limits,
    /// The interrupts the running chunk has taken, and when it began.
    spent: Rc<Cell<u64>>,
    began: Rc<Cell<Instant>>,
    /// The names the caller set, left out of [`Script::globals`].
    injected: HashSet<String>,
}

impl Script {
    /// A sandboxed state with `limits`.
    pub fn new(limits: Limits) -> Result<Self, Error> {
        let host = |e: mlua::Error| Error::Host(e.to_string());
        let libs = StdLib::COROUTINE
            | StdLib::TABLE
            | StdLib::STRING
            | StdLib::UTF8
            | StdLib::BIT
            | StdLib::MATH
            | StdLib::BUFFER;
        let lua = Lua::new_with(libs, LuaOptions::default()).map_err(host)?;
        lua.set_memory_limit(limits.memory).map_err(host)?;
        let globals = lua.globals();
        for name in [
            "load",
            "loadstring",
            "dofile",
            "loadfile",
            "require",
            "getfenv",
            "setfenv",
            "collectgarbage",
            "newproxy",
            "gcinfo",
        ] {
            globals.raw_set(name, Value::Nil).map_err(host)?;
        }
        if limits.allow_load {
            let load = lua
                .create_function(|lua, (text, name): (mlua::LuaString, Option<String>)| {
                    let name = name.unwrap_or_else(|| "=(load)".to_owned());
                    Ok(lua
                        .load(text.as_bytes().to_vec())
                        .set_name(name)
                        .into_function())
                })
                .map_err(host)?;
            globals.raw_set("load", load.clone()).map_err(host)?;
            globals.raw_set("loadstring", load).map_err(host)?;
        }
        if let Some(root) = limits.require_root.clone() {
            let loaded = lua.create_table().map_err(host)?;
            let require = lua
                .create_function(move |lua, name: String| {
                    if let Some(v) = loaded.raw_get::<Option<Value>>(name.as_str())? {
                        return Ok(v);
                    }
                    let path = module_path(&root, &name).ok_or_else(|| {
                        mlua::Error::runtime(format!("require {name}: not a module of the folder"))
                    })?;
                    let text = std::fs::read(&path)
                        .map_err(|e| mlua::Error::runtime(format!("require {name}: {e}")))?;
                    let value: Value = lua.load(text).set_name(format!("={name}")).eval()?;
                    let value = if value.is_nil() {
                        Value::Boolean(true)
                    } else {
                        value
                    };
                    loaded.raw_set(name.as_str(), value.clone())?;
                    Ok(value)
                })
                .map_err(host)?;
            globals.raw_set("require", require).map_err(host)?;
        }
        lua.sandbox(true).map_err(host)?;
        let spent = Rc::new(Cell::new(0_u64));
        let began = Rc::new(Cell::new(Instant::now()));
        let (budget, deadline) = (limits.interrupts, limits.deadline);
        let (s, b) = (Rc::clone(&spent), Rc::clone(&began));
        lua.set_interrupt(move |_| {
            s.set(s.get() + 1);
            if s.get() > budget {
                return Err(mlua::Error::runtime("forge-script: the interrupts' budget"));
            }
            // The clock read every 1024 interrupts.
            if s.get() % 1024 == 0 && b.get().elapsed() > deadline {
                return Err(mlua::Error::runtime("forge-script: the deadline"));
            }
            Ok(VmState::Continue)
        });
        Ok(Self {
            lua,
            limits,
            spent,
            began,
            injected: HashSet::new(),
        })
    }

    /// The caps.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Sets the global `name` to `value` (a stub of another engine's interface: a table, a
    /// number), visible to the chunks run after.
    pub fn set(&mut self, name: &str, value: &Data) -> Result<(), Error> {
        let host = |e: mlua::Error| Error::Host(e.to_string());
        let v = to_lua(&self.lua, value).map_err(host)?;
        self.lua.globals().set(name, v).map_err(host)?;
        self.injected.insert(name.to_owned());
        Ok(())
    }

    /// Sets the global `name` to a Rust function of the chunk's arguments as [`Data`], returning
    /// [`Data`] or an error message: a stub function of another engine's interface.
    pub fn function(
        &mut self,
        name: &str,
        f: impl Fn(Vec<Data>) -> Result<Data, String> + 'static,
    ) -> Result<(), Error> {
        let host = |e: mlua::Error| Error::Host(e.to_string());
        let function = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let args = args
                    .into_iter()
                    .map(|v| to_data(v, 0, &mut HashSet::new()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(mlua::Error::runtime)?;
                let out = f(args).map_err(mlua::Error::runtime)?;
                to_lua(lua, &out)
            })
            .map_err(host)?;
        self.lua.globals().set(name, function).map_err(host)?;
        self.injected.insert(name.to_owned());
        Ok(())
    }

    /// Runs the chunk `source` named `chunk` (its errors name it and their line) and returns what
    /// it returns (`Nil` for nothing).
    pub fn run(&mut self, chunk: &str, source: &str) -> Result<Data, Error> {
        self.spent.set(0);
        self.began.set(Instant::now());
        let result: Result<Value, mlua::Error> =
            self.lua.load(source).set_name(format!("={chunk}")).eval();
        match result {
            Ok(v) => to_data(v, 0, &mut HashSet::new()).map_err(Error::Data),
            Err(e) => Err(self.error(chunk, e)),
        }
    }

    /// The globals the chunks set (not those the caller set), by name.
    pub fn globals(&self) -> Result<BTreeMap<String, Data>, Error> {
        let mut out = BTreeMap::new();
        for pair in self.lua.globals().pairs::<Value, Value>() {
            let (k, v) = pair.map_err(|e| Error::Data(e.to_string()))?;
            let Value::String(k) = k else { continue };
            let k = k.to_string_lossy();
            if self.injected.contains(&k) {
                continue;
            }
            out.insert(k, to_data(v, 0, &mut HashSet::new()).map_err(Error::Data)?);
        }
        Ok(out)
    }

    /// The interrupts the last chunk took.
    pub fn spent(&self) -> u64 {
        self.spent.get()
    }

    /// The memory the state holds, bytes.
    pub fn memory(&self) -> usize {
        self.lua.used_memory()
    }

    fn error(&self, chunk: &str, e: mlua::Error) -> Error {
        let text = e.to_string();
        if text.contains("forge-script: the interrupts' budget") {
            Error::Budget {
                chunk: chunk.to_owned(),
                what: "interrupts",
            }
        } else if text.contains("forge-script: the deadline") {
            Error::Budget {
                chunk: chunk.to_owned(),
                what: "deadline",
            }
        } else if matches!(e, mlua::Error::MemoryError(_)) || text.contains("not enough memory") {
            Error::Memory {
                chunk: chunk.to_owned(),
                cap: self.limits.memory,
            }
        } else if matches!(e, mlua::Error::SyntaxError { .. }) {
            Error::Syntax(text)
        } else {
            Error::Runtime(text)
        }
    }
}

/// The file module `name` names under `root`: its dots or slashes as folders, `.luau` then
/// `.lua`; None for a name that would leave the folder.
fn module_path(root: &Path, name: &str) -> Option<PathBuf> {
    let rel = PathBuf::from(name.replace('.', "/"));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) || name.is_empty() {
        return None;
    }
    ["luau", "lua"]
        .iter()
        .map(|ext| root.join(rel.with_extension(ext)))
        .find(|p| p.is_file())
}

/// `v` as [`Data`]; `seen` holds the tables on the way down (a cycle is refused).
fn to_data(v: Value, depth: usize, seen: &mut HashSet<usize>) -> Result<Data, String> {
    Ok(match v {
        Value::Nil => Data::Nil,
        Value::Boolean(b) => Data::Bool(b),
        Value::Integer(i) => Data::Integer(i),
        Value::Number(n) => number(n),
        Value::String(s) => Data::String(s.to_string_lossy()),
        Value::Table(t) => {
            if depth >= DEEPEST {
                return Err(format!("a table nested deeper than {DEEPEST}"));
            }
            let id = t.to_pointer() as usize;
            if !seen.insert(id) {
                return Err("a table that holds itself".to_owned());
            }
            let mut map = BTreeMap::new();
            for pair in t.pairs::<Value, Value>() {
                let (k, v) = pair.map_err(|e| e.to_string())?;
                let key = match k {
                    Value::Integer(i) => Key::Integer(i),
                    Value::Number(n) => match number(n) {
                        Data::Integer(i) => Key::Integer(i),
                        _ => Key::Other(n.to_string()),
                    },
                    Value::String(s) => Key::String(s.to_string_lossy()),
                    Value::Boolean(b) => Key::Other(b.to_string()),
                    other => Key::Other(other.type_name().to_owned()),
                };
                map.insert(key, to_data(v, depth + 1, seen)?);
            }
            seen.remove(&id);
            let n = map.len() as i64;
            if n > 0
                && map
                    .keys()
                    .enumerate()
                    .all(|(k, key)| *key == Key::Integer(k as i64 + 1))
            {
                Data::List(map.into_values().collect())
            } else if n == 0 {
                Data::Map(BTreeMap::new())
            } else {
                Data::Map(map)
            }
        }
        other => Data::Opaque(other.type_name()),
    })
}

/// A number: an integer when it has no fraction and fits.
fn number(n: f64) -> Data {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Data::Integer(n as i64)
    } else {
        Data::Number(n)
    }
}

/// `d` as a Lua value.
fn to_lua(lua: &Lua, d: &Data) -> mlua::Result<Value> {
    Ok(match d {
        Data::Nil | Data::Opaque(_) => Value::Nil,
        Data::Bool(b) => Value::Boolean(*b),
        Data::Integer(i) => Value::Number(*i as f64),
        Data::Number(n) => Value::Number(*n),
        Data::String(s) => Value::String(lua.create_string(s)?),
        Data::List(items) => {
            let t = lua.create_table()?;
            for (k, item) in items.iter().enumerate() {
                t.raw_set(k + 1, to_lua(lua, item)?)?;
            }
            Value::Table(t)
        }
        Data::Map(map) => {
            let t = lua.create_table()?;
            for (k, v) in map {
                let v = to_lua(lua, v)?;
                match k {
                    Key::Integer(i) => t.raw_set(*i as f64, v)?,
                    Key::String(s) | Key::Other(s) => t.raw_set(s.as_str(), v)?,
                }
            }
            Value::Table(t)
        }
    })
}

#[cfg(test)]
mod tests;

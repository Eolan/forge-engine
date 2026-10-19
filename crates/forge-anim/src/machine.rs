//! A data-driven state machine above the [`Player`] (#312): a character's action states (idle,
//! moving, climbing a fence, attacking) and the ways between them, which the game's variables
//! and posted events open; in each state its animation nodes, the one whose conditions hold
//! most specifically playing; switches through [`Player::play`] (inertialized), with an
//! in-between clip where a transition names one; and events at a node's start, its end or a
//! share of it, which set variables (what sequences a vault's start, its end and the way back)
//! or tell the host (footsteps, sounds).
//!
//! The shape is the one an out-of-tree prototype settled on, running a player of about 125
//! nodes and a pursuer of about 40 from a game's own animation data:
//! - **Ways** out of a state are tried most specific first (the most conditions), then in their
//!   order. Several steps run in one update, but never back to a state already passed in it: a
//!   variable the game sets and the host does not would bounce between two states every frame.
//! - **The node that plays** is the one whose conditions hold with the most conditions (those of
//!   the group that holds, when its conditions are alternatives), then the highest priority, then
//!   the first: priority first let a three-condition sprint vault lose to a one-condition start.
//! - **A gait** (`match_speed`) plays at the character's speed over its natural speed, a blend's
//!   mixed from its clips' by their weights, so the feet keep to the ground; an in-between clip
//!   at its transition's rate. [`Animator::root_motion`] gives what the last update's playing
//!   carried the root by, through blends and in-between clips.
//! - **A rig with fewer clips than the data names** binds with
//!   [`Machine::new_dropping_missing`], which leaves out what it cannot play and says what.
//! - **Inheritance:** a node may extend another (a state's node or a base): the parent's fields,
//!   then the child's, its lists appended (its conditions joined to every group of the parent's),
//!   so a variant is a few lines.
//! - **Events** the game posts ([`Animator::post`]) are seen by the next update, then cleared.
//!   Variables ([`Vars`]) are named without case; a missing one reads false, 0 or "".
//!
//! Built in code ([`MachineDef`]) or from TOML ([`MachineDef::from_toml`]), bound to a rig's
//! clips by name ([`Machine::new`]) and run per character ([`Animator`]). Nothing here calls a
//! transcendental function (D-016).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{BlendSpace, Clip, Player, Pose, RootMotion, Skeleton, Source};

/// What can go wrong building a machine.
#[derive(Debug, thiserror::Error)]
pub enum MachineError {
    /// The TOML did not parse.
    #[error("the machine's TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// A condition's text did not parse.
    #[error("condition {0:?}: {1}")]
    Condition(String, &'static str),
    /// A node names a clip the rig does not have.
    #[error("node {node:?}: no clip {clip:?}")]
    UnknownClip {
        /// The node.
        node: String,
        /// The clip it names.
        clip: String,
    },
    /// A way leads to a state the machine does not have.
    #[error("state {from:?}: no state {to:?} to go to")]
    UnknownState {
        /// The state the way leaves.
        from: String,
        /// The state it names.
        to: String,
    },
    /// A node extends a node the machine does not have, or itself through others.
    #[error("node {0:?}: its parent {1:?} is missing or extends it")]
    Parent(String, String),
    /// A node plays nothing: neither its own clip or blend nor its parents'.
    #[error("node {0:?} plays no clip")]
    NoMotion(String),
    /// The machine has no state.
    #[error("the machine has no state")]
    Empty,
}

/// A variable's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A flag.
    Bool(bool),
    /// A number.
    Number(f32),
    /// A word.
    Text(String),
}

impl Value {
    /// A literal as data writes it: `true` or `false`, a number, else text (without its quotes).
    pub fn parse(text: &str) -> Value {
        let text = text.trim();
        if text.eq_ignore_ascii_case("true") {
            Value::Bool(true)
        } else if text.eq_ignore_ascii_case("false") {
            Value::Bool(false)
        } else if let Ok(x) = text.parse::<f32>() {
            Value::Number(x)
        } else {
            let unquoted = text
                .strip_prefix('\'')
                .and_then(|t| t.strip_suffix('\''))
                .or_else(|| text.strip_prefix('"').and_then(|t| t.strip_suffix('"')))
                .unwrap_or(text);
            Value::Text(unquoted.to_owned())
        }
    }

    /// As a flag: a number other than 0, a word other than "", "0" and "false".
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Number(x) => *x != 0.0,
            Value::Text(t) => !(t.is_empty() || t == "0" || t.eq_ignore_ascii_case("false")),
        }
    }

    /// As a number: a flag's 1 or 0, a word's number or 0.
    pub fn number(&self) -> f32 {
        match self {
            Value::Bool(b) => f32::from(u8::from(*b)),
            Value::Number(x) => *x,
            Value::Text(t) => t.trim().parse().unwrap_or(0.0),
        }
    }

    /// Whether it equals `literal`, compared as the literal's kind: a flag as a flag, a number
    /// as a number to [`NUMBER_EQUAL`], a word without case.
    fn equals(&self, literal: &Value) -> bool {
        match literal {
            Value::Bool(b) => self.truthy() == *b,
            Value::Number(x) => (self.number() - *x).abs() <= NUMBER_EQUAL,
            Value::Text(t) => match self {
                Value::Text(s) => s.eq_ignore_ascii_case(t),
                _ => false,
            },
        }
    }
}

/// How near two numbers count as equal (`name == 0.5`): a value the game computed lands a
/// rounding off the literal the data wrote.
pub const NUMBER_EQUAL: f32 = 1e-4;

/// A character's variables: names compared without case, a missing one reading false, 0 or "".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vars {
    values: BTreeMap<String, Value>,
}

impl Vars {
    /// Sets `name` to `value`.
    pub fn set(&mut self, name: &str, value: Value) {
        self.values.insert(name.to_ascii_lowercase(), value);
    }

    /// Its value, if set.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(&name.to_ascii_lowercase())
    }

    /// Its value as a flag (false when missing).
    pub fn truthy(&self, name: &str) -> bool {
        self.get(name).is_some_and(Value::truthy)
    }

    /// Its value as a number (0 when missing).
    pub fn number(&self, name: &str) -> f32 {
        self.get(name).map_or(0.0, Value::number)
    }

    /// Every variable, its name in lower case, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Applies `Name=value` (a `SetVariable` event's): the value parsed as [`Value::parse`] does.
    pub fn assign(&mut self, assignment: &str) {
        if let Some((name, value)) = assignment.split_once('=') {
            self.set(name.trim(), Value::parse(value));
        }
    }
}

/// One test on the variables or the events posted.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// The variable is true (`name`).
    IsTrue(String),
    /// It is false or missing (`!name`).
    IsFalse(String),
    /// It equals the literal, compared as the literal's kind (`name == literal`).
    Equals(String, Value),
    /// It does not (`name != literal`).
    Differs(String, Value),
    /// It is below the number (`name < x`).
    Below(String, f32),
    /// It is above it (`name > x`).
    Above(String, f32),
    /// The event was posted before this update (`event:name`).
    Event(String),
}

impl Condition {
    /// Parses `name`, `!name`, `name == literal`, `name != literal`, `name < x`, `name > x` or
    /// `event:name`.
    pub fn parse(text: &str) -> Result<Condition, MachineError> {
        let t = text.trim();
        let err = |why| MachineError::Condition(text.to_owned(), why);
        if let Some(name) = t.strip_prefix("event:") {
            return Ok(Condition::Event(name.trim().to_owned()));
        }
        for (op, make) in [
            (
                "==",
                (|n, v| Condition::Equals(n, v)) as fn(String, Value) -> Condition,
            ),
            ("!=", |n, v| Condition::Differs(n, v)),
        ] {
            if let Some((name, value)) = t.split_once(op) {
                return Ok(make(name.trim().to_owned(), Value::parse(value)));
            }
        }
        for (op, below) in [('<', true), ('>', false)] {
            if let Some((name, value)) = t.split_once(op) {
                let x = value
                    .trim()
                    .parse::<f32>()
                    .map_err(|_| err("a comparison takes a number"))?;
                let name = name.trim().to_owned();
                return Ok(if below {
                    Condition::Below(name, x)
                } else {
                    Condition::Above(name, x)
                });
            }
        }
        let (name, negated) = match t.strip_prefix('!') {
            Some(name) => (name.trim(), true),
            None => (t, false),
        };
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(err("a variable's name, alone"));
        }
        Ok(if negated {
            Condition::IsFalse(name.to_owned())
        } else {
            Condition::IsTrue(name.to_owned())
        })
    }

    /// Whether it holds for `vars` and the events posted.
    pub fn holds(&self, vars: &Vars, events: &[String]) -> bool {
        match self {
            Condition::IsTrue(n) => vars.truthy(n),
            Condition::IsFalse(n) => !vars.truthy(n),
            Condition::Equals(n, v) => vars.get(n).unwrap_or(&Value::Text(String::new())).equals(v),
            Condition::Differs(n, v) => {
                !vars.get(n).unwrap_or(&Value::Text(String::new())).equals(v)
            }
            Condition::Below(n, x) => vars.number(n) < *x,
            Condition::Above(n, x) => vars.number(n) > *x,
            Condition::Event(e) => events.iter().any(|p| p.eq_ignore_ascii_case(e)),
        }
    }
}

/// Conditions as alternatives of groups, each group holding when all its conditions do; none
/// at all holds always.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct When {
    /// The groups, any of which may hold.
    pub any: Vec<Vec<Condition>>,
}

impl When {
    /// One group of all these.
    pub fn all(conditions: Vec<Condition>) -> Self {
        Self {
            any: if conditions.is_empty() {
                Vec::new()
            } else {
                vec![conditions]
            },
        }
    }

    /// How specific it holds: the conditions of its largest group that holds (0 with none at
    /// all), `None` when no group does.
    pub fn holds(&self, vars: &Vars, events: &[String]) -> Option<usize> {
        if self.any.is_empty() {
            return Some(0);
        }
        self.any
            .iter()
            .filter(|g| g.iter().all(|c| c.holds(vars, events)))
            .map(Vec::len)
            .max()
    }

    /// Every group of `self` joined to every group of `more` (a child's conditions added to its
    /// parent's), either side's none acting as one empty group.
    fn and(&self, more: &When) -> When {
        if more.any.is_empty() {
            return self.clone();
        }
        if self.any.is_empty() {
            return more.clone();
        }
        When {
            any: self
                .any
                .iter()
                .flat_map(|a| {
                    more.any
                        .iter()
                        .map(move |b| [a.clone(), b.clone()].concat())
                })
                .collect(),
        }
    }
}

/// Where in a node an event fires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum At {
    /// As it starts.
    Start,
    /// As it ends: a one-shot once at its end, a cycle each time it wraps.
    End,
    /// When it passes this share of its cycle (0..1).
    Share(f32),
}

/// An event a node fires: `SetVariable` with `Name=value` sets a variable; every event is also
/// handed to the host ([`Animator::update`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TimedEvent {
    /// When.
    pub at: At,
    /// Its name (`SetVariable`, `Footstep`, `Sound`…).
    pub name: String,
    /// Its parameter.
    pub value: String,
}

/// An event fired by an update.
#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    /// Its name.
    pub name: String,
    /// Its parameter.
    pub value: String,
}

/// What a node plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Motion {
    /// A clip, by name.
    Clip(String),
    /// Clips along a variable ([`BlendSpace`]): each by name with its value, in increasing
    /// order, read at the variable `scalar`.
    Blend {
        /// The variable read.
        scalar: String,
        /// The clips and their values.
        entries: Vec<(String, f32)>,
    },
}

/// A node's way to a node of the next choice: a blend time of its own, and an in-between clip
/// played once before it (idle to run, sprint to stop).
#[derive(Clone, Debug, PartialEq)]
pub struct TransitionDef {
    /// The node it leads to, by name.
    pub to: String,
    /// The switch's blend time, seconds (the target node's own without).
    pub blend: Option<f32>,
    /// The in-between clip, by name.
    pub via: Option<String>,
    /// The in-between clip's playback rate (1 without).
    pub speed: Option<f32>,
}

/// An animation node as data: the fields it leaves out come from its parent ([`Self::extends`])
/// or the defaults (looped, 0.2 s blend, speed 1, priority 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeDef {
    /// Its name.
    pub name: String,
    /// The node it extends (a state's or a base), by name.
    pub extends: Option<String>,
    /// What it plays.
    pub motion: Option<Motion>,
    /// Whether it cycles; a one-shot holds its end and fires [`At::End`] there.
    pub looped: Option<bool>,
    /// The blend time of a switch to it, seconds.
    pub blend: Option<f32>,
    /// Its playback rate (unless it matches the character's speed).
    pub speed: Option<f32>,
    /// A variable holding the character's speed: the node then plays at it over its natural
    /// speed, clamped to 0.5–1.8, in place of its own rate (a gait's stride kept on the ground).
    /// A clip's natural speed is its root's ([`Clip::natural_speed`]); a blend's, its two clips'
    /// mixed by their weights at its variable.
    pub match_speed: Option<String>,
    /// Ties among the nodes whose conditions hold as specifically.
    pub priority: Option<i32>,
    /// When it may play.
    pub when: When,
    /// What it fires.
    pub events: Vec<TimedEvent>,
    /// Its ways to the next node.
    pub transitions: Vec<TransitionDef>,
}

/// A way out of a state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WayDef {
    /// The state it leads to, by name.
    pub to: String,
    /// When it opens.
    pub when: When,
}

/// An action state as data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateDef {
    /// Its name.
    pub name: String,
    /// Its ways out, in order.
    pub ways: Vec<WayDef>,
    /// Its nodes, in order.
    pub nodes: Vec<NodeDef>,
}

/// A state machine as data ([`MachineDef::from_toml`], or built in code).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineDef {
    /// The state a character starts in, by name (the first without).
    pub start: Option<String>,
    /// Nodes that only others extend.
    pub bases: Vec<NodeDef>,
    /// The states.
    pub states: Vec<StateDef>,
}

// The TOML layout: `start`, `[[base]]` nodes, `[[state]]` with `[[state.way]]` and
// `[[state.node]]`. Conditions are written `when = ["a", "!b", "c > 2"]` (one group) or
// `when = [["a"], ["b", "c"]]` (alternatives).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileDef {
    start: Option<String>,
    #[serde(default)]
    base: Vec<NodeFile>,
    #[serde(default)]
    state: Vec<StateFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateFile {
    name: String,
    #[serde(default)]
    way: Vec<WayFile>,
    #[serde(default)]
    node: Vec<NodeFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WayFile {
    to: String,
    #[serde(default)]
    when: WhenFile,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WhenFile {
    All(Vec<String>),
    Any(Vec<Vec<String>>),
}

impl Default for WhenFile {
    fn default() -> Self {
        WhenFile::All(Vec::new())
    }
}

impl WhenFile {
    fn parse(self) -> Result<When, MachineError> {
        let group = |g: Vec<String>| g.iter().map(|c| Condition::parse(c)).collect();
        Ok(match self {
            WhenFile::All(g) => When::all(group(g)?),
            WhenFile::Any(gs) => When {
                any: gs.into_iter().map(group).collect::<Result<_, _>>()?,
            },
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlendFile {
    scalar: String,
    entries: Vec<(String, f32)>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AtFile {
    Share(f32),
    Named(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventFile {
    at: AtFile,
    name: String,
    #[serde(default)]
    value: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionFile {
    to: String,
    blend: Option<f32>,
    via: Option<String>,
    speed: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeFile {
    name: String,
    extends: Option<String>,
    clip: Option<String>,
    blend_space: Option<BlendFile>,
    looped: Option<bool>,
    blend: Option<f32>,
    speed: Option<f32>,
    match_speed: Option<String>,
    priority: Option<i32>,
    #[serde(default)]
    when: WhenFile,
    #[serde(default)]
    events: Vec<EventFile>,
    #[serde(default)]
    transitions: Vec<TransitionFile>,
}

impl NodeFile {
    fn parse(self) -> Result<NodeDef, MachineError> {
        let motion = match (self.clip, self.blend_space) {
            (Some(clip), _) => Some(Motion::Clip(clip)),
            (None, Some(b)) => Some(Motion::Blend {
                scalar: b.scalar,
                entries: b.entries,
            }),
            (None, None) => None,
        };
        let events = self
            .events
            .into_iter()
            .map(|e| {
                let at = match e.at {
                    AtFile::Share(s) => At::Share(s),
                    AtFile::Named(n) if n.eq_ignore_ascii_case("start") => At::Start,
                    AtFile::Named(n) if n.eq_ignore_ascii_case("end") => At::End,
                    AtFile::Named(n) => {
                        return Err(MachineError::Condition(
                            n,
                            "an event is at start, end or a share",
                        ));
                    }
                };
                Ok(TimedEvent {
                    at,
                    name: e.name,
                    value: e.value,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(NodeDef {
            name: self.name,
            extends: self.extends,
            motion,
            looped: self.looped,
            blend: self.blend,
            speed: self.speed,
            match_speed: self.match_speed,
            priority: self.priority,
            when: self.when.parse()?,
            events,
            transitions: self
                .transitions
                .into_iter()
                .map(|t| TransitionDef {
                    to: t.to,
                    blend: t.blend,
                    via: t.via,
                    speed: t.speed,
                })
                .collect(),
        })
    }
}

impl MachineDef {
    /// Reads a machine from TOML: `start = "idle"`, `[[base]]` nodes others extend, and
    /// `[[state]]` tables (`name`) with their `[[state.way]]` (`to`, `when`) and
    /// `[[state.node]]` (`name`, `extends`, `clip` or `blend_space = { scalar, entries }`,
    /// `looped`, `blend`, `speed`, `match_speed`, `priority`, `when`, `events = [{ at, name,
    /// value }]` with `at` "start", "end" or a share, `transitions = [{ to, blend, via }]`).
    pub fn from_toml(text: &str) -> Result<MachineDef, MachineError> {
        let file: FileDef = toml::from_str(text)?;
        Ok(MachineDef {
            start: file.start,
            bases: file
                .base
                .into_iter()
                .map(NodeFile::parse)
                .collect::<Result<_, _>>()?,
            states: file
                .state
                .into_iter()
                .map(|s| {
                    Ok(StateDef {
                        name: s.name,
                        ways: s
                            .way
                            .into_iter()
                            .map(|w| {
                                Ok(WayDef {
                                    to: w.to,
                                    when: w.when.parse()?,
                                })
                            })
                            .collect::<Result<_, MachineError>>()?,
                        nodes: s
                            .node
                            .into_iter()
                            .map(NodeFile::parse)
                            .collect::<Result<_, _>>()?,
                    })
                })
                .collect::<Result<_, MachineError>>()?,
        })
    }

    /// Node `node` with its parents' fields and lists: the root's first, each child's after.
    fn resolved(&self, node: &NodeDef) -> Result<NodeDef, MachineError> {
        let find = |name: &str| {
            self.bases
                .iter()
                .chain(self.states.iter().flat_map(|s| &s.nodes))
                .find(|n| n.name.eq_ignore_ascii_case(name))
        };
        // The chain from `node` up, bounded by the nodes there are (a longer one loops).
        let total = self.bases.len() + self.states.iter().map(|s| s.nodes.len()).sum::<usize>();
        let mut chain = vec![node];
        while let Some(parent) = chain.last().and_then(|n| n.extends.as_deref()) {
            let found = find(parent)
                .filter(|_| chain.len() <= total)
                .ok_or_else(|| MachineError::Parent(node.name.clone(), parent.to_owned()))?;
            chain.push(found);
        }
        let mut out = NodeDef::default();
        for n in chain.iter().rev() {
            out.name.clone_from(&n.name);
            out.motion = n.motion.clone().or(out.motion);
            out.looped = n.looped.or(out.looped);
            out.blend = n.blend.or(out.blend);
            out.speed = n.speed.or(out.speed);
            out.match_speed = n.match_speed.clone().or(out.match_speed);
            out.priority = n.priority.or(out.priority);
            out.when = out.when.and(&n.when);
            out.events.extend(n.events.iter().cloned());
            out.transitions.extend(n.transitions.iter().cloned());
        }
        out.extends = None;
        Ok(out)
    }
}

/// A node bound to its clips.
#[derive(Clone, Debug)]
struct Node {
    name: String,
    source: Source,
    looped: bool,
    blend: f32,
    speed: f32,
    /// The variable holding the character's speed ([`NodeDef::match_speed`]).
    match_speed: Option<String>,
    /// Its clip's, or each of its blend's entries', root travel over a cycle (metres) and cycles
    /// a second.
    natural: Vec<[f32; 2]>,
    /// A blend node's variable.
    scalar: Option<String>,
    priority: i32,
    when: When,
    events: Vec<TimedEvent>,
    transitions: Vec<Transition>,
}

impl Node {
    /// Its playback rate with `vars`: the character's speed over its natural speed, clamped, or
    /// its own rate (without a speed to match, or a natural speed of 0). A blend's natural speed
    /// is its root's as it plays: its two clips' travel over a cycle mixed by their weights, times
    /// their cycles a second mixed the same way ([`BlendSpace::rate`]).
    fn rate(&self, vars: &Vars) -> f32 {
        let [travel, cycles] = match &self.source {
            Source::Clip(_) => self.natural.first().copied().unwrap_or([0.0; 2]),
            Source::Blend(space) => {
                let parameter = self.scalar.as_deref().map_or(0.0, |s| vars.number(s));
                let (i, j, w) = space.around(parameter);
                let (a, b) = (self.natural[i], self.natural[j]);
                [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w]
            }
        };
        let natural = travel * cycles;
        match &self.match_speed {
            Some(var) if natural > 0.0 => {
                (vars.number(var) / natural).clamp(MATCH_SPEED.0, MATCH_SPEED.1)
            }
            _ => self.speed,
        }
    }
}

#[derive(Clone, Debug)]
struct Transition {
    to: String,
    blend: Option<f32>,
    via: Option<usize>,
    speed: f32,
}

/// What [`Machine::new_dropping_missing`] left out: a node one of whose clips the rig lacks (or a
/// blend's entry, the rest of its entries kept), or a transition's in-between clip (the
/// transition kept without it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dropped {
    /// The node.
    pub node: String,
    /// The clip the rig lacks.
    pub clip: String,
    /// What went: the node, one of its blend's entries, or a transition's in-between clip.
    pub what: DroppedPart,
}

/// What of a node [`Dropped`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DroppedPart {
    /// The node, its clip missing (or all its blend's).
    Node,
    /// One entry of its blend.
    BlendEntry,
    /// A transition's in-between clip.
    InBetween,
}

#[derive(Clone, Debug)]
struct Way {
    to: usize,
    when: When,
}

#[derive(Clone, Debug)]
struct State {
    name: String,
    ways: Vec<Way>,
    nodes: Vec<Node>,
}

/// A machine bound to a rig's clips ([`Machine::new`]): what every character running it
/// ([`Animator`]) reads.
#[derive(Clone, Debug)]
pub struct Machine {
    states: Vec<State>,
    start: usize,
}

/// The clip of `clips` named `name`: the same name first, else the same without case.
fn clip_index(clips: &[Clip], name: &str) -> Option<usize> {
    clips
        .iter()
        .position(|c| c.name == name)
        .or_else(|| clips.iter().position(|c| c.name.eq_ignore_ascii_case(name)))
}

impl Machine {
    /// `def` bound to `clips` (a rig's), their names resolved; `root` is the skeleton's joint
    /// whose travel gives a clip's natural speed ([`NodeDef::match_speed`]). A clip the rig lacks
    /// is an error ([`Self::new_dropping_missing`] leaves its node out instead).
    pub fn new(
        def: &MachineDef,
        clips: &[Clip],
        skeleton: &Skeleton,
        root: usize,
    ) -> Result<Machine, MachineError> {
        Self::bind(def, clips, skeleton, root, false).map(|(machine, _)| machine)
    }

    /// [`Self::new`], leaving out what names a clip the rig lacks, and saying what: a node (a
    /// blend's entry, the node kept with the rest), or a transition's in-between clip (the
    /// transition kept without it). A game's data names hundreds of clips and one rig carries a
    /// few dozen of them.
    pub fn new_dropping_missing(
        def: &MachineDef,
        clips: &[Clip],
        skeleton: &Skeleton,
        root: usize,
    ) -> Result<(Machine, Vec<Dropped>), MachineError> {
        Self::bind(def, clips, skeleton, root, true)
    }

    fn bind(
        def: &MachineDef,
        clips: &[Clip],
        skeleton: &Skeleton,
        root: usize,
        drop_missing: bool,
    ) -> Result<(Machine, Vec<Dropped>), MachineError> {
        if def.states.is_empty() {
            return Err(MachineError::Empty);
        }
        let state_index = |from: &str, to: &str| {
            def.states
                .iter()
                .position(|s| s.name.eq_ignore_ascii_case(to))
                .ok_or_else(|| MachineError::UnknownState {
                    from: from.to_owned(),
                    to: to.to_owned(),
                })
        };
        let missing = |node: &str, name: &str| MachineError::UnknownClip {
            node: node.to_owned(),
            clip: name.to_owned(),
        };
        // A clip's root travel over a cycle and its cycles a second.
        let natural = |c: usize| {
            let d = clips[c].duration;
            if d > 0.0 {
                [clips[c].natural_speed(skeleton, root) * d, 1.0 / d]
            } else {
                [0.0; 2]
            }
        };
        let mut dropped = Vec::new();
        let mut states = Vec::with_capacity(def.states.len());
        for s in &def.states {
            let ways = s
                .ways
                .iter()
                .map(|w| {
                    Ok(Way {
                        to: state_index(&s.name, &w.to)?,
                        when: w.when.clone(),
                    })
                })
                .collect::<Result<_, MachineError>>()?;
            let mut nodes = Vec::with_capacity(s.nodes.len());
            for n in &s.nodes {
                let n = def.resolved(n)?;
                let (source, scalar, naturals) = match &n.motion {
                    Some(Motion::Clip(name)) => match clip_index(clips, name) {
                        Some(c) => (Source::Clip(c), None, vec![natural(c)]),
                        None if drop_missing => {
                            dropped.push(Dropped {
                                node: n.name.clone(),
                                clip: name.clone(),
                                what: DroppedPart::Node,
                            });
                            continue;
                        }
                        None => return Err(missing(&n.name, name)),
                    },
                    Some(Motion::Blend { scalar, entries }) => {
                        let mut kept = Vec::with_capacity(entries.len());
                        for (name, value) in entries {
                            match clip_index(clips, name) {
                                Some(c) => kept.push((c, *value)),
                                None if drop_missing => dropped.push(Dropped {
                                    node: n.name.clone(),
                                    clip: name.clone(),
                                    what: DroppedPart::BlendEntry,
                                }),
                                None => return Err(missing(&n.name, name)),
                            }
                        }
                        if kept.is_empty() {
                            dropped.push(Dropped {
                                node: n.name.clone(),
                                clip: String::new(),
                                what: DroppedPart::Node,
                            });
                            continue;
                        }
                        let naturals = kept.iter().map(|&(c, _)| natural(c)).collect();
                        (
                            Source::Blend(BlendSpace { entries: kept }),
                            Some(scalar.clone()),
                            naturals,
                        )
                    }
                    None => return Err(MachineError::NoMotion(n.name.clone())),
                };
                let mut transitions = Vec::with_capacity(n.transitions.len());
                for t in &n.transitions {
                    let via = match t.via.as_deref() {
                        None => None,
                        Some(name) => match clip_index(clips, name) {
                            Some(c) => Some(c),
                            None if drop_missing => {
                                dropped.push(Dropped {
                                    node: n.name.clone(),
                                    clip: name.to_owned(),
                                    what: DroppedPart::InBetween,
                                });
                                None
                            }
                            None => return Err(missing(&n.name, name)),
                        },
                    };
                    transitions.push(Transition {
                        to: t.to.clone(),
                        blend: t.blend,
                        via,
                        speed: t.speed.unwrap_or(1.0),
                    });
                }
                nodes.push(Node {
                    name: n.name.clone(),
                    source,
                    looped: n.looped.unwrap_or(true),
                    blend: n.blend.unwrap_or(0.2),
                    speed: n.speed.unwrap_or(1.0),
                    match_speed: n.match_speed.clone(),
                    natural: naturals,
                    scalar,
                    priority: n.priority.unwrap_or(0),
                    when: n.when.clone(),
                    events: n.events.clone(),
                    transitions,
                });
            }
            states.push(State {
                name: s.name.clone(),
                ways,
                nodes,
            });
        }
        let start = match &def.start {
            Some(name) => state_index("start", name)?,
            None => 0,
        };
        Ok((Machine { states, start }, dropped))
    }

    /// Its states' names, in order.
    pub fn state_names(&self) -> impl Iterator<Item = &str> {
        self.states.iter().map(|s| s.name.as_str())
    }
}

/// The match-speed rate's bounds: a stride played slower than half or faster than 1.8 times its
/// own reads as sliding.
const MATCH_SPEED: (f32, f32) = (0.5, 1.8);

/// One character running a [`Machine`]: its variables, the events posted for the next update,
/// its state and node, and the [`Player`] that plays them.
#[derive(Clone, Debug)]
pub struct Animator {
    /// Its variables, which the game and the nodes' `SetVariable` events set.
    pub vars: Vars,
    posted: Vec<String>,
    state: usize,
    /// The node playing (or the one an in-between clip leads to): its state and its index there.
    /// A state without a node of its own keeps the one before playing.
    node: Option<(usize, usize)>,
    /// An in-between clip plays before `node`, at this rate.
    between: bool,
    between_speed: f32,
    /// Where the last update's motion started in the source playing: the phase it was at, or
    /// `None` when the source started in that update (from its start).
    motion_from: Option<f32>,
    /// The source was switched in this update.
    switched: bool,
    player: Player,
}

/// Events posted for one update, at most: the rest are dropped (a host posting every frame
/// without updating would grow the list without end).
pub const MAX_POSTED: usize = 32;

impl Animator {
    /// A character of `joints` joints at `machine`'s start, playing nothing until its first
    /// update chooses a node.
    pub fn new(machine: &Machine, joints: usize) -> Self {
        Self {
            vars: Vars::default(),
            posted: Vec::new(),
            state: machine.start,
            node: None,
            between: false,
            between_speed: 1.0,
            motion_from: None,
            switched: false,
            player: Player::new(Source::Clip(0), joints),
        }
    }

    /// Posts `event` for the next update's conditions (`event:name`); it is cleared after. The
    /// same event twice counts once, and past [`MAX_POSTED`] events are dropped.
    pub fn post(&mut self, event: &str) {
        if self.posted.len() < MAX_POSTED && !self.posted.iter().any(|p| p == event) {
            self.posted.push(event.to_owned());
        }
    }

    /// The player under it: its source (a node's clip or blend, or an in-between clip), its
    /// phase and the parameter a blend reads.
    pub fn player(&self) -> &Player {
        &self.player
    }

    /// How the root moved over the last update ([`Clip::root_motion`]): along what plays from
    /// the phase it was at to the one it reached (from its start when it started in that
    /// update), a blend's clips' motions mixed by their weights; an in-between clip's too. For a
    /// controller that moves the character by its animation (#311); `root` is the skeleton's
    /// root joint.
    pub fn root_motion(&self, clips: &[Clip], skeleton: &Skeleton, root: usize) -> RootMotion {
        let to = self.player.phase();
        let from = self.motion_from.unwrap_or(0.0);
        let along = |c: usize| {
            let d = clips[c].duration;
            clips[c].root_motion(skeleton, root, from * d, to * d)
        };
        match self.player.source() {
            Source::Clip(c) => along(*c),
            Source::Blend(space) => {
                let (i, j, w) = space.around(self.player.parameter);
                let a = along(space.entries[i].0);
                if j == i || w <= 0.0 {
                    a
                } else {
                    a.mix(along(space.entries[j].0), w)
                }
            }
        }
    }

    /// Its state's name.
    pub fn state<'m>(&self, machine: &'m Machine) -> &'m str {
        &machine.states[self.state].name
    }

    /// Its node's name ("" before the first update), the phase reached and whether an
    /// in-between clip plays before it: what the host times things by (a climb carries the body
    /// over the first 35 % of the vault's end).
    pub fn node<'m>(&self, machine: &'m Machine) -> (&'m str, f32, bool) {
        let name = self
            .node
            .map_or("", |(s, n)| machine.states[s].nodes[n].name.as_str());
        (name, self.player.phase(), self.between)
    }

    /// Whether a one-shot node (or an in-between clip) has reached its end.
    pub fn finished(&self) -> bool {
        self.player.finished()
    }

    /// One line on where it stands: its state, its node and phase, and its variables (the live
    /// view a console command or the overlay prints).
    pub fn describe(&self, machine: &Machine) -> String {
        let (node, phase, between) = self.node(machine);
        let vars: Vec<String> = self
            .vars
            .iter()
            .map(|(k, v)| match v {
                Value::Bool(b) => format!("{k}={b}"),
                Value::Number(x) => format!("{k}={x}"),
                Value::Text(t) => format!("{k}={t:?}"),
            })
            .collect();
        format!(
            "{} / {node}{} at {phase:.2}; {}",
            self.state(machine),
            if between { " (in between)" } else { "" },
            vars.join(", ")
        )
    }

    /// Moves on by `dt` seconds: takes the ways the variables and the events posted open, then
    /// the node whose conditions hold most specifically, switches to it (through its
    /// transition's in-between clip when it names one), and writes the pose into `out`. Returns
    /// the events fired, `SetVariable`'s already applied.
    pub fn update(
        &mut self,
        machine: &Machine,
        clips: &[Clip],
        skeleton: &Skeleton,
        dt: f32,
        out: &mut Pose,
    ) -> Vec<Fired> {
        let events = std::mem::take(&mut self.posted);
        let mut fired = Vec::new();
        self.switched = false;
        // The states: the most specific way that holds (the first of those as specific), never
        // back to a state passed in this update.
        let mut passed = vec![self.state];
        loop {
            let next = machine.states[self.state]
                .ways
                .iter()
                .enumerate()
                .filter(|(_, w)| !passed.contains(&w.to))
                .filter_map(|(i, w)| w.when.holds(&self.vars, &events).map(|n| (n, i, w.to)))
                .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            let Some((.., to)) = next else {
                break;
            };
            self.state = to;
            passed.push(to);
        }
        // The node: the most conditions, then the highest priority, then the first; the one
        // playing when none of the state's holds and it is the state's, else the state's first.
        let state = &machine.states[self.state];
        let chosen = state
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let count = n.when.holds(&self.vars, &events)?;
                Some((count, n.priority, i))
            })
            .max_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(b.2.cmp(&a.2)))
            .map(|(.., i)| (self.state, i))
            .or(self.node.filter(|&(s, _)| s == self.state))
            .or((!state.nodes.is_empty()).then_some((self.state, 0)))
            .or(self.node);
        if let Some(c) = chosen
            && self.node != Some(c)
        {
            let target = &machine.states[c.0].nodes[c.1];
            let transition = self.node.and_then(|(s, n)| {
                machine.states[s].nodes[n]
                    .transitions
                    .iter()
                    .find(|t| t.to.eq_ignore_ascii_case(&target.name))
            });
            let blend = transition.and_then(|t| t.blend).unwrap_or(target.blend);
            self.node = Some(c);
            match transition.and_then(|t| t.via.map(|via| (via, t.speed))) {
                Some((via, speed)) => {
                    self.player
                        .play_once(Source::Clip(via), blend, clips, skeleton);
                    self.between = true;
                    self.between_speed = speed;
                    self.switched = true;
                }
                None => {
                    self.start(target, blend, clips, skeleton);
                    self.fire(target, At::Start, &mut fired);
                }
            }
        }
        let Some((s, n)) = self.node else {
            return fired;
        };
        let node = &machine.states[s].nodes[n];
        // An in-between clip played out: its node, with its own blend.
        if self.between && self.player.finished() {
            self.start(node, node.blend, clips, skeleton);
            self.fire(node, At::Start, &mut fired);
        }
        if self.between {
            self.player.speed = self.between_speed;
        } else {
            self.player.speed = node.rate(&self.vars);
            if let Some(scalar) = &node.scalar {
                self.player.parameter = self.vars.number(scalar);
            }
        }
        let (phase0, done0) = (self.player.phase(), self.player.finished());
        self.motion_from = (!self.switched).then_some(phase0);
        self.player.update(clips, skeleton, dt, out);
        if self.between {
            return fired;
        }
        // The shares passed: (phase0, phase1], across the wrap when it cycled.
        let phase1 = self.player.phase();
        let wrapped = node.looped && phase1 < phase0;
        let crossed = |s: f32| {
            if wrapped {
                s > phase0 || s <= phase1
            } else {
                s > phase0 && s <= phase1
            }
        };
        for e in &node.events {
            if let At::Share(share) = e.at
                && crossed(share)
            {
                emit(&mut self.vars, e, &mut fired);
            }
        }
        if wrapped || (!node.looped && self.player.finished() && !done0) {
            self.fire(node, At::End, &mut fired);
            if wrapped {
                self.fire(node, At::Start, &mut fired);
            }
        }
        fired
    }

    /// Switches the player to `node` over `blend` seconds.
    fn start(&mut self, node: &Node, blend: f32, clips: &[Clip], skeleton: &Skeleton) {
        self.between = false;
        self.switched = true;
        if node.looped {
            self.player
                .play(node.source.clone(), blend, clips, skeleton);
        } else {
            self.player
                .play_once(node.source.clone(), blend, clips, skeleton);
        }
    }

    /// Fires `node`'s events at `at`.
    fn fire(&mut self, node: &Node, at: At, fired: &mut Vec<Fired>) {
        for e in node.events.iter().filter(|e| e.at == at) {
            emit(&mut self.vars, e, fired);
        }
    }
}

/// Fires `e`: a `SetVariable` sets its variable in `vars`; every event goes to the host.
fn emit(vars: &mut Vars, e: &TimedEvent, fired: &mut Vec<Fired>) {
    if e.name.eq_ignore_ascii_case("SetVariable") {
        vars.assign(&e.value);
    }
    fired.push(Fired {
        name: e.name.clone(),
        value: e.value.clone(),
    });
}

#[cfg(test)]
mod tests {
    use glam::{Mat4, Vec3};

    use super::*;
    use crate::clip::{Interpolation, JointTracks, Track};
    use crate::pose::Transform;

    fn skeleton() -> Skeleton {
        Skeleton::new(
            vec!["root".into()],
            vec![None],
            vec![Mat4::IDENTITY],
            &[Transform::IDENTITY],
            vec![Mat4::IDENTITY],
        )
        .unwrap()
    }

    /// A clip named `name` of `duration` seconds whose root walks `travel` metres along x.
    fn clip(name: &str, duration: f32, travel: f32) -> Clip {
        Clip {
            name: name.into(),
            start: 0.0,
            duration,
            joints: vec![JointTracks {
                translation: Some(Track {
                    times: vec![0.0, duration],
                    values: vec![Vec3::ZERO, Vec3::X * travel],
                    interpolation: Interpolation::Linear,
                }),
                rotation: None,
                scale: None,
            }],
        }
    }

    fn clips() -> Vec<Clip> {
        vec![
            clip("Idle", 2.0, 0.0),
            clip("Walk", 1.0, 1.5),
            clip("Run", 0.5, 2.0),
            clip("IdleToRun", 0.3, 0.2),
            clip("VaultStart", 0.5, 0.5),
            clip("VaultEnd", 1.0, 1.0),
        ]
    }

    /// The prototype's shape in small: idle, moving (a walk, a run through an in-between clip,
    /// a vault whose start and end sequence themselves by `SetVariable`), and the way back.
    const MACHINE: &str = r#"
        start = "idle"

        [[base]]
        name = "locomotion"
        looped = true
        blend = 0.25
        match_speed = "Speed"

        [[state]]
        name = "idle"
        way = [{ to = "moving", when = ["isMoving"] }]
        [[state.node]]
        name = "idle"
        clip = "Idle"
        transitions = [{ to = "run", blend = 0.1, via = "IdleToRun" }]

        [[state]]
        name = "moving"
        way = [
            { to = "idle", when = ["!isMoving"] },
            { to = "vault", when = ["event:Vault"] },
        ]
        [[state.node]]
        name = "walk"
        extends = "locomotion"
        clip = "Walk"
        events = [{ at = 0.5, name = "Footstep", value = "left" }]
        [[state.node]]
        name = "run"
        extends = "walk"
        clip = "Run"
        when = ["isRunning"]
        priority = 1

        [[state]]
        name = "vault"
        way = [{ to = "idle", when = ["Finished"] }]
        [[state.node]]
        name = "start"
        clip = "VaultStart"
        looped = false
        events = [{ at = "end", name = "SetVariable", value = "Started=true" }]
        [[state.node]]
        name = "end"
        clip = "VaultEnd"
        looped = false
        when = ["Started"]
        events = [{ at = 0.7, name = "SetVariable", value = "Finished=true" }]
    "#;

    fn machine() -> (Machine, Vec<Clip>, Skeleton) {
        let (clips, skeleton) = (clips(), skeleton());
        let def = MachineDef::from_toml(MACHINE).expect("the machine parses");
        let machine = Machine::new(&def, &clips, &skeleton, 0).expect("the machine binds");
        (machine, clips, skeleton)
    }

    /// Runs `animator` for `steps` steps of `dt`, gathering what fired.
    fn run(
        animator: &mut Animator,
        (machine, clips, skeleton): &(Machine, Vec<Clip>, Skeleton),
        steps: usize,
        dt: f32,
    ) -> Vec<Fired> {
        let mut pose = skeleton.rest().clone();
        (0..steps)
            .flat_map(|_| animator.update(machine, clips, skeleton, dt, &mut pose))
            .collect()
    }

    #[test]
    fn conditions_parse_and_read_missing_variables_as_nothing() {
        let c = |t: &str| Condition::parse(t).unwrap();
        assert_eq!(c("a"), Condition::IsTrue("a".into()));
        assert_eq!(c("!a"), Condition::IsFalse("a".into()));
        assert_eq!(
            c("w == 'Rifle'"),
            Condition::Equals("w".into(), Value::Text("Rifle".into()))
        );
        assert_eq!(
            c("n != 2"),
            Condition::Differs("n".into(), Value::Number(2.0))
        );
        assert_eq!(c("s > 1.5"), Condition::Above("s".into(), 1.5));
        assert_eq!(c("s < 1"), Condition::Below("s".into(), 1.0));
        assert_eq!(c("event:Shove"), Condition::Event("Shove".into()));
        assert!(Condition::parse("two words").is_err());
        assert!(Condition::parse("s > fast").is_err());
        let mut vars = Vars::default();
        // Missing: false, 0, "".
        assert!(c("!a").holds(&vars, &[]) && !c("a").holds(&vars, &[]));
        assert!(c("s < 1").holds(&vars, &[]) && c("w == ''").holds(&vars, &[]));
        // Names and words without case.
        vars.set("Weapon", Value::Text("rifle".into()));
        assert!(c("weapon == Rifle").holds(&vars, &[]));
        vars.assign("Started=true");
        assert!(c("started").holds(&vars, &[]));
        assert!(c("event:shove").holds(&vars, &["Shove".into()]));
        // Alternatives: the most specific group that holds.
        let when = When {
            any: vec![vec![c("a")], vec![c("started"), c("weapon == rifle")]],
        };
        assert_eq!(when.holds(&vars, &[]), Some(2));
        assert_eq!(When::default().holds(&vars, &[]), Some(0));
    }

    #[test]
    fn a_child_takes_its_parents_fields_and_adds_its_own() {
        let def = MachineDef::from_toml(MACHINE).unwrap();
        let run = &def.states[1].nodes[1];
        let resolved = def.resolved(run).unwrap();
        // From the base through the walk: looped, its blend and speed variable, the walk's
        // footstep; its own clip, condition and priority.
        assert_eq!(resolved.looped, Some(true));
        assert_eq!(resolved.blend, Some(0.25));
        assert_eq!(resolved.match_speed.as_deref(), Some("Speed"));
        assert_eq!(resolved.events.len(), 1);
        assert_eq!(resolved.motion, Some(Motion::Clip("Run".into())));
        assert_eq!(resolved.priority, Some(1));
        assert_eq!(resolved.when.any.len(), 1);
        // A loop in the parents is an error, not a hang.
        let mut looped = def.clone();
        looped.bases[0].extends = Some("run".into());
        assert!(matches!(
            looped.resolved(&looped.states[1].nodes[1]),
            Err(MachineError::Parent(..))
        ));
    }

    #[test]
    fn binding_names_what_is_missing() {
        let (clips, skeleton) = (clips(), skeleton());
        let mut def = MachineDef::from_toml(MACHINE).unwrap();
        def.states[0].nodes[0].motion = Some(Motion::Clip("Sleep".into()));
        assert!(matches!(
            Machine::new(&def, &clips, &skeleton, 0),
            Err(MachineError::UnknownClip { .. })
        ));
        let mut def = MachineDef::from_toml(MACHINE).unwrap();
        def.states[0].ways[0].to = "nowhere".into();
        assert!(matches!(
            Machine::new(&def, &clips, &skeleton, 0),
            Err(MachineError::UnknownState { .. })
        ));
        assert!(MachineDef::from_toml("[[state]]\nname = 1").is_err());
    }

    #[test]
    fn the_most_specific_node_plays_before_the_higher_priority() {
        let m = machine();
        let mut a = Animator::new(&m.0, 1);
        a.vars.set("isMoving", Value::Bool(true));
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert_eq!(a.state(&m.0), "moving");
        assert_eq!(a.node(&m.0).0, "walk");
        // The run's one condition beats the walk's none.
        a.vars.set("isRunning", Value::Bool(true));
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert_eq!(a.node(&m.0).0, "run");
        // Specificity before priority: a node of two conditions and priority 0 beats one of one
        // condition and priority 9.
        let (clips, skeleton) = (clips(), skeleton());
        let mut def = MachineDef::from_toml(MACHINE).unwrap();
        def.states[1].nodes[1].priority = Some(9);
        def.states[1].nodes.push(NodeDef {
            name: "sprint".into(),
            motion: Some(Motion::Clip("Run".into())),
            when: When::all(vec![
                Condition::IsTrue("isRunning".into()),
                Condition::IsTrue("isSprinting".into()),
            ]),
            ..NodeDef::default()
        });
        let machine = Machine::new(&def, &clips, &skeleton, 0).unwrap();
        let m = (machine, clips, skeleton);
        let mut a = Animator::new(&m.0, 1);
        for v in ["isMoving", "isRunning", "isSprinting"] {
            a.vars.set(v, Value::Bool(true));
        }
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert_eq!(a.node(&m.0).0, "sprint");
    }

    #[test]
    fn a_transition_plays_its_in_between_clip_first() {
        let m = machine();
        let mut a = Animator::new(&m.0, 1);
        run(&mut a, &m, 2, 1.0 / 60.0);
        assert_eq!(a.node(&m.0), ("idle", a.node(&m.0).1, false));
        a.vars.set("isMoving", Value::Bool(true));
        a.vars.set("isRunning", Value::Bool(true));
        run(&mut a, &m, 1, 1.0 / 60.0);
        let (node, _, between) = a.node(&m.0);
        assert_eq!((node, between), ("run", true));
        // IdleToRun lasts 0.3 s: the run takes over after it.
        run(&mut a, &m, 30, 1.0 / 60.0);
        assert!(!a.node(&m.0).2);
        assert_eq!(a.node(&m.0).0, "run");
    }

    #[test]
    fn ways_never_bounce_back_within_an_update() {
        // Two states whose ways both hold: one step, not a bounce every frame.
        let text = r#"
            [[state]]
            name = "a"
            way = [{ to = "b", when = ["x"] }]
            [[state.node]]
            name = "a"
            clip = "Idle"
            [[state]]
            name = "b"
            way = [{ to = "a", when = ["x"] }]
            [[state.node]]
            name = "b"
            clip = "Walk"
        "#;
        let (clips, skeleton) = (clips(), skeleton());
        let def = MachineDef::from_toml(text).unwrap();
        let machine = Machine::new(&def, &clips, &skeleton, 0).unwrap();
        let m = (machine, clips, skeleton);
        let mut a = Animator::new(&m.0, 1);
        a.vars.set("x", Value::Bool(true));
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert_eq!(a.state(&m.0), "b");
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert_eq!(a.state(&m.0), "a");
    }

    #[test]
    fn a_vault_sequences_itself_by_its_events() {
        let m = machine();
        let mut a = Animator::new(&m.0, 1);
        a.vars.set("isMoving", Value::Bool(true));
        run(&mut a, &m, 2, 1.0 / 60.0);
        assert_eq!(a.state(&m.0), "moving");
        // The posted event opens the way once, then is cleared.
        a.post("Vault");
        let fired = run(&mut a, &m, 1, 1.0 / 60.0);
        assert!(fired.is_empty());
        assert_eq!((a.state(&m.0), a.node(&m.0).0), ("vault", "start"));
        // The start (0.5 s) sets Started at its end, which picks the end node; the end sets
        // Finished at 70 % of its second, which takes the action back to idle, and on to moving
        // in the same update while the character still moves.
        let fired = run(&mut a, &m, 40, 1.0 / 60.0);
        assert!(fired.iter().any(|f| f.value == "Started=true"));
        assert_eq!(a.node(&m.0).0, "end");
        let fired = run(&mut a, &m, 50, 1.0 / 60.0);
        assert!(fired.iter().any(|f| f.value == "Finished=true"));
        assert_eq!(a.state(&m.0), "moving");
        // Stopped, the same vault ends in idle.
        let mut b = Animator::new(&m.0, 1);
        b.vars.set("isMoving", Value::Bool(true));
        run(&mut b, &m, 2, 1.0 / 60.0);
        b.post("Vault");
        run(&mut b, &m, 1, 1.0 / 60.0);
        assert_eq!(b.state(&m.0), "vault");
        b.vars.set("isMoving", Value::Bool(false));
        run(&mut b, &m, 90, 1.0 / 60.0);
        assert_eq!(b.state(&m.0), "idle");
        assert!(
            b.describe(&m.0).starts_with("idle / idle at"),
            "{}",
            b.describe(&m.0)
        );
    }

    #[test]
    fn a_cycle_fires_its_events_each_time_round() {
        let m = machine();
        let mut a = Animator::new(&m.0, 1);
        a.vars.set("isMoving", Value::Bool(true));
        // The walk's natural speed is 1.5 m/s: at 1.5 it plays as authored, one cycle a second,
        // and its footstep at half of it fires once a second.
        a.vars.set("Speed", Value::Number(1.5));
        let fired = run(&mut a, &m, 241, 1.0 / 60.0);
        let steps = fired.iter().filter(|f| f.name == "Footstep").count();
        assert_eq!(steps, 4);
        // Twice as fast a character: the walk at 1.8 times (the clamp), 7 steps in 4 s.
        let mut b = Animator::new(&m.0, 1);
        b.vars.set("isMoving", Value::Bool(true));
        b.vars.set("Speed", Value::Number(3.0));
        let fired = run(&mut b, &m, 241, 1.0 / 60.0);
        let steps = fired.iter().filter(|f| f.name == "Footstep").count();
        assert_eq!(steps, 7);
    }

    /// A machine of one state with one node, `node`, over the test clips.
    fn one_node(node: NodeDef) -> (Machine, Vec<Clip>, Skeleton) {
        let (clips, skeleton) = (clips(), skeleton());
        let def = MachineDef {
            start: None,
            bases: Vec::new(),
            states: vec![StateDef {
                name: "moving".into(),
                ways: Vec::new(),
                nodes: vec![node],
            }],
        };
        let machine = Machine::new(&def, &clips, &skeleton, 0).unwrap();
        (machine, clips, skeleton)
    }

    #[test]
    fn a_blend_matches_the_speed_its_clips_mix_to() {
        // The walk carries the root 1.5 m/s, the run 4 m/s: at either end the blend plays the
        // clip's natural speed as authored, at 6 m/s the run alone 1.5 times, at 0.3 the clamp.
        let m = one_node(NodeDef {
            name: "gait".into(),
            motion: Some(Motion::Blend {
                scalar: "Speed".into(),
                entries: vec![("Walk".into(), 1.5), ("Run".into(), 4.0)],
            }),
            match_speed: Some("Speed".into()),
            speed: Some(0.92),
            ..NodeDef::default()
        });
        let mut a = Animator::new(&m.0, 1);
        for (speed, rate) in [(4.0, 1.0), (6.0, 1.5), (1.5, 1.0), (0.3, 0.5)] {
            a.vars.set("Speed", Value::Number(speed));
            run(&mut a, &m, 2, 1.0 / 60.0);
            assert!(
                (a.player().speed - rate).abs() < 1e-5,
                "at {speed} m/s: {}",
                a.player().speed
            );
        }
        // Its root moves at the character's speed: a second at 2.75 m/s carries it 2.75 m.
        a.vars.set("Speed", Value::Number(2.75));
        let mut pose = m.2.rest().clone();
        let mut travel = 0.0;
        for _ in 0..60 {
            a.update(&m.0, &m.1, &m.2, 1.0 / 60.0, &mut pose);
            travel += a.root_motion(&m.1, &m.2, 0).translation.x;
        }
        assert!((travel - 2.75).abs() < 0.02, "{travel} m in a second");
    }

    #[test]
    fn an_in_between_clip_plays_at_its_transitions_rate() {
        let text = MACHINE.replace(
            r#"transitions = [{ to = "run", blend = 0.1, via = "IdleToRun" }]"#,
            r#"transitions = [{ to = "run", blend = 0.1, via = "IdleToRun", speed = 2.0 }]"#,
        );
        let (clips, skeleton) = (clips(), skeleton());
        let def = MachineDef::from_toml(&text).unwrap();
        let machine = Machine::new(&def, &clips, &skeleton, 0).unwrap();
        let m = (machine, clips, skeleton);
        let mut a = Animator::new(&m.0, 1);
        run(&mut a, &m, 2, 1.0 / 60.0);
        a.vars.set("isMoving", Value::Bool(true));
        a.vars.set("isRunning", Value::Bool(true));
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert!(a.node(&m.0).2);
        assert_eq!(a.player().speed, 2.0);
        // IdleToRun's 0.3 s at twice its rate: done in 0.15 s, where the first test's took 0.3.
        run(&mut a, &m, 12, 1.0 / 60.0);
        assert!(!a.node(&m.0).2, "{}", a.describe(&m.0));
    }

    #[test]
    fn a_rig_without_some_clips_binds_without_their_nodes() {
        let (clips, skeleton) = (clips(), skeleton());
        let mut def = MachineDef::from_toml(MACHINE).unwrap();
        def.states[1].nodes.push(NodeDef {
            name: "sneak".into(),
            motion: Some(Motion::Clip("Sneak".into())),
            ..NodeDef::default()
        });
        def.states[1].nodes.push(NodeDef {
            name: "gait".into(),
            motion: Some(Motion::Blend {
                scalar: "Speed".into(),
                entries: vec![
                    ("Walk".into(), 1.5),
                    ("Jog".into(), 2.5),
                    ("Run".into(), 4.0),
                ],
            }),
            ..NodeDef::default()
        });
        def.states[0].nodes[0].transitions[0].via = Some("Stumble".into());
        assert!(Machine::new(&def, &clips, &skeleton, 0).is_err());
        let (machine, dropped) = Machine::new_dropping_missing(&def, &clips, &skeleton, 0).unwrap();
        let parts: Vec<(&str, &str, DroppedPart)> = dropped
            .iter()
            .map(|d| (d.node.as_str(), d.clip.as_str(), d.what))
            .collect();
        assert_eq!(
            parts,
            [
                ("idle", "Stumble", DroppedPart::InBetween),
                ("sneak", "Sneak", DroppedPart::Node),
                ("gait", "Jog", DroppedPart::BlendEntry),
            ]
        );
        // The rest runs: idle to run straight, without the in-between clip it lost.
        let m = (machine, clips, skeleton);
        let mut a = Animator::new(&m.0, 1);
        run(&mut a, &m, 2, 1.0 / 60.0);
        a.vars.set("isMoving", Value::Bool(true));
        a.vars.set("isRunning", Value::Bool(true));
        run(&mut a, &m, 1, 1.0 / 60.0);
        assert_eq!(a.node(&m.0), ("run", a.node(&m.0).1, false));
    }

    #[test]
    fn numbers_compare_to_a_rounding_and_posted_events_are_bounded() {
        let mut vars = Vars::default();
        vars.set("lean", Value::Number(0.500_04));
        assert!(Condition::parse("lean == 0.5").unwrap().holds(&vars, &[]));
        assert!(!Condition::parse("lean == 0.51").unwrap().holds(&vars, &[]));
        let m = machine();
        let mut a = Animator::new(&m.0, 1);
        for k in 0..100 {
            a.post(&format!("E{}", k % 50));
            a.post("Vault");
        }
        assert_eq!(a.posted.len(), MAX_POSTED);
    }
}

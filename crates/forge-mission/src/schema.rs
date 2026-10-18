//! The mission record as written (RON, D-035, D-073): objectives, places, items, counters and one
//! tree of steps.
//!
//! ```ron
//! Mission(
//!     id: "lab:fetch",
//!     version: 2,
//!     objectives: [(id: "climb", text: "Climb the stairs")],
//!     places: [(id: "top", shape: Box(min: (6.0, 1.0, -1.5), max: (6.6, 3.0, 1.5)))],
//!     steps: (id: "climb", objective: "climb", until: Inside("top")),
//! )
//! ```
//!
//! A step is one of: a wait (`until`, and `fail_if`), `seq`, `all`, `race`, `branch` or `do`
//! (actions). Every step may show an `objective`, be `optional`, and run actions when it
//! succeeds (`then`), fails (`on_fail`) or is cancelled (`on_cancel`). Optional fields are
//! written without `Some(..)`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Error;

/// The schema's version a record is written in ([`Mission::version`]).
pub const SCHEMA_VERSION: u32 = 2;

/// The id of a root step written without one.
pub const ROOT: &str = "root";

/// A mission: what the player is asked, where, with what, and in which order (D-072, D-073).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    /// Its id, namespaced as content ids are (D-035): `package:name`.
    pub id: String,
    /// The schema's version it is written in: [`SCHEMA_VERSION`].
    pub version: u32,
    /// What the player is shown, each named by one step ([`Step::objective`]).
    pub objectives: Vec<Objective>,
    /// The places the conditions name.
    #[serde(default)]
    pub places: Vec<Place>,
    /// The items the steps spawn and the player picks up.
    #[serde(default)]
    pub items: Vec<Item>,
    /// The counters and flags the steps set and test, with their values at the start.
    #[serde(default)]
    pub vars: BTreeMap<String, i64>,
    /// The steps, from the mission's start.
    pub steps: Step,
    /// Steps renamed since an earlier version, old id to new: a save of the earlier version
    /// finds them (D-073's reconcile rule). An old id is never used again.
    #[serde(default)]
    pub renamed: BTreeMap<String, String>,
}

/// An objective: an id a step names and the text the player reads. Its status is its step's
/// ([`crate::Status`]).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    /// Unique in its mission.
    pub id: String,
    /// What the player is asked.
    pub text: String,
}

/// A place: a volume of the world, metres, the y axis up.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    /// Unique among the mission's places and items.
    pub id: String,
    /// Its volume.
    pub shape: Shape,
}

/// A place's volume.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub enum Shape {
    /// An axis-aligned box, its corners.
    Box {
        /// The lowest corner.
        min: [f64; 3],
        /// The highest corner.
        max: [f64; 3],
    },
    /// A ball.
    Sphere {
        /// Its middle.
        centre: [f64; 3],
        /// Its radius, metres.
        radius: f64,
    },
}

impl Shape {
    /// Whether `point` is inside, its edge included.
    pub fn contains(&self, point: [f64; 3]) -> bool {
        match *self {
            Self::Box { min, max } => (0..3).all(|k| min[k] <= point[k] && point[k] <= max[k]),
            Self::Sphere { centre, radius } => {
                let d: f64 = (0..3).map(|k| (point[k] - centre[k]).powi(2)).sum();
                d <= radius * radius
            }
        }
    }

    /// Its middle, where a walker heads for it.
    pub fn centre(&self) -> [f64; 3] {
        match *self {
            Self::Box { min, max } => std::array::from_fn(|k| 0.5 * (min[k] + max[k])),
            Self::Sphere { centre, .. } => centre,
        }
    }
}

/// An item: absent until an [`Action::Spawn`] puts it at its place, then picked up by the
/// player coming within its reach, and held from then on.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Unique among the mission's places and items.
    pub id: String,
    /// Where it lies, metres.
    pub at: [f64; 3],
    /// How near the player's feet come to pick it up, metres.
    #[serde(default = "default_reach")]
    pub reach: f64,
}

fn default_reach() -> f64 {
    1.0
}

/// A step of the mission's tree: exactly one of `until` (with or without `fail_if`; a wait),
/// `fail_if` alone (a wait that never succeeds), `seq`, `all`, `race`, `branch` and `do`.
///
/// The combinators are Verse's structured concurrency's (`docs/research/missions.md` §3):
/// - **`seq`:** each in turn; it fails when a required step fails.
/// - **`all`:** all at once; it succeeds when its required steps have, fails when one fails, and
///   cancels what still runs when it ends (its optional steps are then *missed*).
/// - **`race`:** all at once; the first to end decides it, failed or not (Verse's rule), ties in
///   the written order, and the others are cancelled.
/// - **`branch`:** its step started in the background; the branch itself succeeds at once, and
///   its step is cancelled when the step holding the branch ends.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Its id, unique in the mission. Without one, its parent's id and its place among its
    /// siblings (`main/2`), the root's [`ROOT`]: name the steps a later edit may move, so saves
    /// find them.
    #[serde(default)]
    pub id: Option<String>,
    /// The objective shown while it runs: completed when it succeeds, failed when it fails,
    /// missed when it is cancelled.
    #[serde(default)]
    pub objective: Option<String>,
    /// Its failure does not fail its parent, and an `all` does not wait for it.
    #[serde(default)]
    pub optional: bool,
    /// A wait: it succeeds when this holds.
    #[serde(default)]
    pub until: Option<Condition>,
    /// A wait: it fails when this holds, tested first (a tie goes to failure).
    #[serde(default)]
    pub fail_if: Option<Condition>,
    /// Each in turn.
    #[serde(default)]
    pub seq: Option<Vec<Step>>,
    /// All at once, until the required ones have succeeded.
    #[serde(default)]
    pub all: Option<Vec<Step>>,
    /// All at once, until the first ends.
    #[serde(default)]
    pub race: Option<Vec<Step>>,
    /// In the background.
    #[serde(default)]
    pub branch: Option<Box<Step>>,
    /// Actions, run in order; the step succeeds in the same tick.
    #[serde(default, rename = "do")]
    pub actions: Option<Vec<Action>>,
    /// Run when it succeeds.
    #[serde(default)]
    pub then: Vec<Action>,
    /// Run when it fails.
    #[serde(default)]
    pub on_fail: Vec<Action>,
    /// Run when it is cancelled.
    #[serde(default)]
    pub on_cancel: Vec<Action>,
}

/// What a wait tests, each tick from its start. Conditions are on the world's state (D-073);
/// `Event` stays for what has no state yet. Every part, once it holds, stays held for that wait,
/// so `AllOf` asks for each to have happened, not for all at once.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Condition {
    /// The player's feet in the place.
    Inside(String),
    /// The player's feet within this many metres of a place's middle or of an item lying at its
    /// place.
    Near(String, f64),
    /// The item picked up (at any time).
    Holds(String),
    /// This many seconds since the step began.
    After(f64),
    /// A counter compares.
    Var(String, Compare),
    /// An event the game sends by name since the step began (`Runtime::notify`): a kill, a door
    /// opened, until those systems have state to test.
    Event(String),
    /// All of them, each once.
    AllOf(Vec<Condition>),
    /// Any one of them.
    AnyOf(Vec<Condition>),
    /// It does not hold.
    Not(Box<Condition>),
    /// Never: a wait that only `fail_if` or a cancellation ends.
    Never,
}

/// A counter's test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum Compare {
    /// At least this.
    AtLeast(i64),
    /// At most this.
    AtMost(i64),
    /// Exactly this.
    Equals(i64),
}

impl Compare {
    /// Whether `value` passes.
    pub fn holds(self, value: i64) -> bool {
        match self {
            Self::AtLeast(v) => value >= v,
            Self::AtMost(v) => value <= v,
            Self::Equals(v) => value == v,
        }
    }
}

/// What a step does: a `do` step's actions, and a step's `then`, `on_fail` and `on_cancel`.
/// They apply in order, and a step later in the same tick sees them.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Action {
    /// The item put at its place.
    Spawn(String),
    /// A counter set.
    Set(String, i64),
    /// A counter changed by this much.
    Add(String, i64),
    /// A mission event for the interface, the sound and the game (`MissionEvent::Emitted`):
    /// a namespaced tag, `mission/thing`.
    Emit(String),
    /// The mission ended at once, won or lost.
    End(End),
}

/// How an [`Action::End`] ends the mission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum End {
    /// Won.
    Succeeded,
    /// Lost.
    Failed,
}

/// The mission in `text` (RON, optional fields written without `Some`), checked: its version,
/// its ids unique, every id it names defined, its numbers finite and its sizes positive.
pub fn parse(text: &str) -> Result<Mission, Error> {
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    let mission: Mission = options
        .from_str(text)
        .map_err(|e| Error::Parse(e.to_string()))?;
    validate(&mission)?;
    Ok(mission)
}

/// Every step's id in the tree's order (depth first, a step before its children): its own, or
/// its parent's and its place (`main/2`), the root's [`ROOT`].
pub fn step_ids(root: &Step) -> Vec<String> {
    fn walk(step: &Step, id: String, out: &mut Vec<String>) {
        out.push(id.clone());
        for (k, child) in children(step).iter().enumerate() {
            let own = child.id.clone().unwrap_or_else(|| format!("{id}/{k}"));
            walk(child, own, out);
        }
    }
    let mut out = Vec::new();
    walk(
        root,
        root.id.clone().unwrap_or_else(|| ROOT.to_owned()),
        &mut out,
    );
    out
}

/// A step's children in their order: a combinator's steps, a branch's step.
pub fn children(step: &Step) -> Vec<&Step> {
    if let Some(s) = step
        .seq
        .as_ref()
        .or(step.all.as_ref())
        .or(step.race.as_ref())
    {
        s.iter().collect()
    } else if let Some(b) = &step.branch {
        vec![b]
    } else {
        Vec::new()
    }
}

/// Checks a mission as [`parse`] does.
pub fn validate(m: &Mission) -> Result<(), Error> {
    let invalid = |what: String| Err(Error::Invalid(format!("{}: {what}", m.id)));
    if !m.id.contains(':') {
        return invalid("the id is not namespaced (package:name)".to_owned());
    }
    if m.version != SCHEMA_VERSION {
        return invalid(format!(
            "schema version {} (this build reads {SCHEMA_VERSION}; version 1's records are \
             written again by hand: the steps became (until: ..), (seq: [..]) and so on)",
            m.version
        ));
    }
    let objectives: Vec<&String> = m.objectives.iter().map(|o| &o.id).collect();
    let things: Vec<&String> = m
        .places
        .iter()
        .map(|p| &p.id)
        .chain(m.items.iter().map(|i| &i.id))
        .collect();
    for (kind, ids) in [("objective", &objectives), ("place or item", &things)] {
        for (n, id) in ids.iter().enumerate() {
            if ids[..n].contains(id) {
                return invalid(format!("the {kind} {id} is defined twice"));
            }
        }
    }
    for place in &m.places {
        let fine = match place.shape {
            Shape::Box { min, max } => {
                (0..3).all(|k| min[k].is_finite() && max[k].is_finite() && min[k] <= max[k])
            }
            Shape::Sphere { centre, radius } => {
                centre.iter().all(|v| v.is_finite()) && radius.is_finite() && radius > 0.0
            }
        };
        if !fine {
            return invalid(format!("the place {} has no proper volume", place.id));
        }
    }
    for item in &m.items {
        if !item.at.iter().all(|v| v.is_finite()) || !(item.reach.is_finite() && item.reach > 0.0) {
            return invalid(format!("the item {} has no proper place or reach", item.id));
        }
    }
    let ids = step_ids(&m.steps);
    for (n, id) in ids.iter().enumerate() {
        if ids[..n].contains(id) {
            return invalid(format!("two steps are {id}"));
        }
    }
    for (old, new) in &m.renamed {
        if ids.contains(old) {
            return invalid(format!("the renamed step {old} is a step's id again"));
        }
        if !ids.contains(new) {
            return invalid(format!("{old} is renamed {new}, which no step is"));
        }
    }
    let mut named = Vec::new();
    let mut at = 0;
    check_step(m, &m.steps, &ids, &mut at, &mut named)
}

fn check_step<'m>(
    m: &'m Mission,
    step: &'m Step,
    ids: &[String],
    at: &mut usize,
    named: &mut Vec<&'m str>,
) -> Result<(), Error> {
    let id = &ids[*at];
    *at += 1;
    let invalid = |what: String| Err(Error::Invalid(format!("{}: the step {id}: {what}", m.id)));
    let kinds = [
        step.until.is_some() || step.fail_if.is_some(),
        step.seq.is_some(),
        step.all.is_some(),
        step.race.is_some(),
        step.branch.is_some(),
        step.actions.is_some(),
    ];
    match kinds.iter().filter(|&&k| k).count() {
        1 => {}
        0 => return invalid("it is none of a wait, seq, all, race, branch or do".to_owned()),
        _ => return invalid("it is two kinds of step at once".to_owned()),
    }
    if let Some(o) = &step.objective {
        if !m.objectives.iter().any(|x| &x.id == o) {
            return invalid(format!("the objective {o} is not defined"));
        }
        if named.contains(&o.as_str()) {
            return invalid(format!("the objective {o} is named by two steps"));
        }
        named.push(o);
    }
    for c in step.until.iter().chain(&step.fail_if) {
        check_condition(m, c).or_else(invalid)?;
    }
    for a in step
        .actions
        .iter()
        .flatten()
        .chain(&step.then)
        .chain(&step.on_fail)
        .chain(&step.on_cancel)
    {
        check_action(m, a).or_else(invalid)?;
    }
    if let Some(all) = &step.all
        && all.iter().all(|s| s.optional)
    {
        return invalid("an all needs a step that is not optional".to_owned());
    }
    if let Some(race) = &step.race {
        if race.is_empty() {
            return invalid("a race needs a step".to_owned());
        }
        if race.iter().any(|s| s.optional) {
            return invalid(
                "a race's steps cannot be optional: the first to end decides".to_owned(),
            );
        }
    }
    for child in children(step) {
        check_step(m, child, ids, at, named)?;
    }
    Ok(())
}

fn check_condition(m: &Mission, c: &Condition) -> Result<(), String> {
    let place = |id: &String| m.places.iter().any(|p| &p.id == id);
    let item = |id: &String| m.items.iter().any(|i| &i.id == id);
    match c {
        Condition::Inside(id) if !place(id) => Err(format!("the place {id} is not defined")),
        Condition::Holds(id) if !item(id) => Err(format!("the item {id} is not defined")),
        Condition::Near(id, _) if !place(id) && !item(id) => {
            Err(format!("no place or item is {id}"))
        }
        Condition::Near(_, metres) if !(metres.is_finite() && *metres > 0.0) => {
            Err(format!("near within {metres} m"))
        }
        Condition::After(seconds) if !(seconds.is_finite() && *seconds >= 0.0) => {
            Err(format!("a wait of {seconds} s"))
        }
        Condition::Var(name, _) if !m.vars.contains_key(name) => {
            Err(format!("the counter {name} is not declared"))
        }
        Condition::AllOf(cs) | Condition::AnyOf(cs) => {
            cs.iter().try_for_each(|c| check_condition(m, c))
        }
        Condition::Not(c) => check_condition(m, c),
        _ => Ok(()),
    }
}

fn check_action(m: &Mission, a: &Action) -> Result<(), String> {
    match a {
        Action::Spawn(id) if !m.items.iter().any(|i| &i.id == id) => {
            Err(format!("the item {id} is not defined"))
        }
        Action::Set(name, _) | Action::Add(name, _) if !m.vars.contains_key(name) => {
            Err(format!("the counter {name} is not declared"))
        }
        _ => Ok(()),
    }
}

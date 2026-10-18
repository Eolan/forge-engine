//! The mission record as written (RON, D-035): objectives, places, items and one tree of steps.
//!
//! ```ron
//! Mission(
//!     id: "lab:fetch",
//!     version: 1,
//!     objectives: [(id: "climb", text: "Climb the stairs")],
//!     places: [(id: "top", shape: Box(min: (6.0, 1.0, -1.5), max: (6.6, 3.0, 1.5)))],
//!     steps: Objective("climb", Wait(Entered("top"))),
//! )
//! ```

use serde::{Deserialize, Serialize};

use crate::Error;

/// The schema's version a record is written in ([`Mission::version`]).
pub const SCHEMA_VERSION: u32 = 1;

/// A mission: what the player is asked, where, with what, and in which order (D-072).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    /// Its id, namespaced as content ids are (D-035): `package:name`.
    pub id: String,
    /// The schema's version it is written in: [`SCHEMA_VERSION`].
    pub version: u32,
    /// What the player is shown, each named once in the steps by [`Step::Objective`].
    pub objectives: Vec<Objective>,
    /// The places the triggers name.
    #[serde(default)]
    pub places: Vec<Place>,
    /// The items the steps spawn and the player picks up.
    #[serde(default)]
    pub items: Vec<Item>,
    /// The steps, from the mission's start.
    pub steps: Step,
}

/// An objective: an id the steps name and the text the player reads.
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
    /// Unique in its mission.
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

/// An item: absent until a [`Step::Spawn`] puts it at its place, then picked up by the player
/// coming within its reach.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Unique in its mission.
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

/// A step of the mission's tree. The combinators are Verse's structured concurrency's
/// (`docs/research/scripting.md`): `Sequence` one after the other, `All` together until every
/// one is done (Verse's `sync`), `Race` together until the first is done, the others cancelled,
/// `Branch` in the background until its scope ends.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Step {
    /// Done when the trigger fires.
    Wait(Trigger),
    /// Each in turn.
    Sequence(Vec<Step>),
    /// All at once, done when all are.
    All(Vec<Step>),
    /// All at once, done when the first is; the others are cancelled.
    Race(Vec<Step>),
    /// Started in the background: the steps after it go on at once, and it is cancelled when the
    /// step holding it ends.
    Branch(Box<Step>),
    /// The objective shown while its step runs, completed when it is done.
    Objective(String, Box<Step>),
    /// The item put at its place.
    Spawn(String),
    /// The mission won.
    Succeed,
    /// The mission lost.
    Fail,
}

/// What a [`Step::Wait`] waits for. Once a trigger has fired during its wait it stays fired, so
/// `AllOf` asks for each to have happened, not for all at once.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub enum Trigger {
    /// The player's feet in the place.
    Entered(String),
    /// The item picked up (at any time).
    PickedUp(String),
    /// This many seconds since the wait began.
    After(f64),
    /// An event the game sends by name since the wait began (`Runtime::notify`): a kill, a
    /// door opened, until those systems have their own triggers.
    Event(String),
    /// All of them, each once.
    AllOf(Vec<Trigger>),
    /// Any one of them.
    AnyOf(Vec<Trigger>),
}

/// The mission in `text` (RON), checked: its version, its ids unique, every id it names
/// defined, its numbers finite and its sizes positive.
pub fn parse(text: &str) -> Result<Mission, Error> {
    let mission: Mission = ron::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
    validate(&mission)?;
    Ok(mission)
}

/// Checks a mission as [`parse`] does.
pub fn validate(m: &Mission) -> Result<(), Error> {
    let invalid = |what: String| Err(Error::Invalid(format!("{}: {what}", m.id)));
    if !m.id.contains(':') {
        return invalid("the id is not namespaced (package:name)".to_owned());
    }
    if m.version != SCHEMA_VERSION {
        return invalid(format!(
            "schema version {} (this build reads {SCHEMA_VERSION})",
            m.version
        ));
    }
    for (kind, ids) in [
        (
            "objective",
            m.objectives.iter().map(|o| &o.id).collect::<Vec<_>>(),
        ),
        ("place", m.places.iter().map(|p| &p.id).collect()),
        ("item", m.items.iter().map(|i| &i.id).collect()),
    ] {
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
    let mut named = Vec::new();
    check_step(m, &m.steps, &mut named)?;
    Ok(())
}

fn check_step<'m>(m: &'m Mission, step: &'m Step, named: &mut Vec<&'m str>) -> Result<(), Error> {
    let invalid = |what: String| Err(Error::Invalid(format!("{}: {what}", m.id)));
    match step {
        Step::Wait(t) => check_trigger(m, t),
        Step::Sequence(steps) | Step::All(steps) | Step::Race(steps) => {
            steps.iter().try_for_each(|s| check_step(m, s, named))
        }
        Step::Branch(s) => check_step(m, s, named),
        Step::Objective(id, s) => {
            if !m.objectives.iter().any(|o| &o.id == id) {
                return invalid(format!("the objective {id} is not defined"));
            }
            if named.contains(&id.as_str()) {
                return invalid(format!("the objective {id} is named by two steps"));
            }
            named.push(id);
            check_step(m, s, named)
        }
        Step::Spawn(id) => {
            if m.items.iter().any(|i| &i.id == id) {
                Ok(())
            } else {
                invalid(format!("the item {id} is not defined"))
            }
        }
        Step::Succeed | Step::Fail => Ok(()),
    }
}

fn check_trigger(m: &Mission, trigger: &Trigger) -> Result<(), Error> {
    let invalid = |what: String| Err(Error::Invalid(format!("{}: {what}", m.id)));
    match trigger {
        Trigger::Entered(id) if !m.places.iter().any(|p| &p.id == id) => {
            invalid(format!("the place {id} is not defined"))
        }
        Trigger::PickedUp(id) if !m.items.iter().any(|i| &i.id == id) => {
            invalid(format!("the item {id} is not defined"))
        }
        Trigger::After(seconds) if !(seconds.is_finite() && *seconds >= 0.0) => {
            invalid(format!("a wait of {seconds} s"))
        }
        Trigger::AllOf(ts) | Trigger::AnyOf(ts) => ts.iter().try_for_each(|t| check_trigger(m, t)),
        _ => Ok(()),
    }
}

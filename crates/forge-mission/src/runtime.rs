//! A mission evaluated at the fixed tick (D-072, D-073): its steps compiled into a flat tree in
//! their written order, each step's state, the items, the counters. The objectives are a view
//! of their steps. The state is saved exactly (a snapshot: rollback, replays, digests) or by
//! stage (which steps ended and how, by id: save games and hot reload, which load into a changed
//! record).
//!
//! Each tick (D-073's total order):
//! 1. the events the game sent are dated, and the items within the player's reach are picked up;
//! 2. the tree is updated from its root, one depth-first pass in the written order. A wait tests
//!    `fail_if`, then `until`. A step that ends tells its parent at once, so a `seq` starts its
//!    next step in the same pass, and a chain of `do` steps takes no time;
//! 3. a step's actions apply in order, and the steps after it in the pass see them;
//! 4. when the mission ends, everything still running is cancelled, in the tree's order.

use std::collections::BTreeMap;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::schema::{
    Action, Compare, Condition, End, Item, Mission, Objective, Step, children, step_ids,
};

/// No condition.
const NONE: u32 = u32::MAX;

/// A step's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Its conditions' roots ([`NONE`] for none).
    Wait {
        until: u32,
        fail_if: u32,
    },
    Seq,
    All,
    Race,
    Branch,
    Do,
}

/// An action, its names resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Spawn(u16),
    Set(u16, i64),
    Add(u16, i64),
    Emit(String),
    End(End),
}

#[derive(Clone, Debug)]
struct StepNode {
    kind: Kind,
    children: Vec<u32>,
    objective: Option<u16>,
    optional: bool,
    actions: Vec<Act>,
    then: Vec<Act>,
    on_fail: Vec<Act>,
    on_cancel: Vec<Act>,
}

/// What a condition near tests against.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Place(u16),
    Item(u16),
}

/// A node of a wait's condition.
#[derive(Clone, Copy, Debug, PartialEq)]
enum CondKind {
    Inside(u16),
    /// The target and the distance squared.
    Near(Target, f64),
    Holds(u16),
    /// Ticks since the step began.
    After(u64),
    Var(u16, Compare),
    /// The event's index among the names the mission uses.
    Event(u16),
    AllOf,
    AnyOf,
    Not,
    Never,
}

#[derive(Clone, Debug)]
struct CondNode {
    kind: CondKind,
    children: Vec<u32>,
}

/// Where a step stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle = 0,
    Running = 1,
    Succeeded = 2,
    Failed = 3,
    Cancelled = 4,
}

impl Phase {
    fn of(byte: u8) -> Option<Self> {
        [
            Self::Idle,
            Self::Running,
            Self::Succeeded,
            Self::Failed,
            Self::Cancelled,
        ]
        .get(usize::from(byte))
        .copied()
    }

    fn ended(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// A step's state: its phase, the tick it began, and a `seq`'s step in progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StepState {
    phase: Phase,
    since: u64,
    cursor: u32,
}

/// An objective's status: its step's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Its step has not begun.
    Pending,
    /// Shown: its step runs.
    Active,
    /// Its step succeeded.
    Completed,
    /// Its step failed.
    Failed,
    /// Its step was cancelled (a race lost, an `all` ended, the mission ended): shown as
    /// missed, not failed (D-073).
    Missed,
}

/// How a step stands ([`Runtime::step`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    /// Not begun.
    Idle,
    /// Running.
    Running,
    /// Ended, succeeded.
    Succeeded,
    /// Ended, failed.
    Failed,
    /// Ended, cancelled.
    Cancelled,
}

/// How the mission stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Still going.
    #[default]
    Running,
    /// Won: its root step succeeded, or an `End(Succeeded)`.
    Succeeded,
    /// Lost: its root step failed, or an `End(Failed)`.
    Failed,
}

impl Outcome {
    fn of(byte: u8) -> Option<Self> {
        [Self::Running, Self::Succeeded, Self::Failed]
            .get(usize::from(byte))
            .copied()
    }
}

/// An item's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    /// Not spawned yet.
    Absent,
    /// At its place, to pick up.
    Present,
    /// Picked up at that tick.
    Picked {
        /// The tick.
        tick: u64,
    },
}

/// What a tick changed, for the interface, the sound and the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionEvent {
    /// An objective is shown.
    ObjectiveStarted(String),
    /// An objective is done.
    ObjectiveCompleted(String),
    /// An objective failed.
    ObjectiveFailed(String),
    /// An objective was missed (its step cancelled).
    ObjectiveMissed(String),
    /// An item appeared at its place.
    ItemSpawned(String),
    /// The player picked an item up.
    ItemPicked(String),
    /// A step's `Emit`: its tag.
    Emitted(String),
    /// The mission is won.
    Succeeded,
    /// The mission is lost.
    Failed,
}

/// How a step ended, in a [`Stage`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ended {
    /// It succeeded.
    Succeeded,
    /// It failed.
    Failed,
    /// It was cancelled.
    Cancelled,
}

/// A mission's progress as a save game keeps it, by id. It loads into the mission's record even
/// after the record changed (hot reload, a newer version: D-073's reconcile rule):
/// - the steps that ended, still in the record (their ids followed through
///   [`Mission::renamed`]), stay ended without running their actions again;
/// - the steps that were running start again, their timers too;
/// - the counters keep their values by name (new ones start at their values, removed ones are
///   dropped), and the items their state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    /// The mission's id.
    pub mission: String,
    /// The steps that ended, by id.
    pub ended: BTreeMap<String, Ended>,
    /// The counters.
    pub vars: BTreeMap<String, i64>,
    /// The items spawned and not yet picked up.
    pub present: Vec<String>,
    /// The items picked up.
    pub picked: Vec<String>,
    /// How it stands.
    pub outcome: Outcome,
}

/// A mission running: its record compiled, and its state.
#[derive(Clone, Debug)]
pub struct Runtime {
    mission: Mission,
    /// The record's hash: a snapshot loads only into the record it was taken from.
    hash: u64,
    tick_rate: u32,
    steps: Vec<StepNode>,
    /// Each step's id, in the tree's order.
    ids: Vec<String>,
    conds: Vec<CondNode>,
    /// The event names the conditions use.
    names: Vec<String>,
    /// The counters' names, in the record's order.
    var_names: Vec<String>,
    /// Per objective, the step that shows it.
    objective_step: Vec<u32>,
    state: Vec<StepState>,
    /// Per condition node: held since its wait began.
    latches: Vec<bool>,
    items: Vec<ItemState>,
    vars: Vec<i64>,
    /// Per event name, the last tick it was sent.
    last_event: Vec<Option<u64>>,
    /// The events sent since the last tick.
    pending: Vec<u16>,
    outcome: Outcome,
    /// What the last tick changed.
    events: Vec<MissionEvent>,
}

impl Runtime {
    /// The mission at its start, with `tick_rate` ticks a second (its seconds become ticks,
    /// rounded up). The record must be valid ([`crate::schema::validate`]).
    pub fn new(mission: Mission, tick_rate: u32) -> Result<Self, Error> {
        crate::schema::validate(&mission)?;
        let hash = fnv(ron::to_string(&mission)
            .map_err(|e| Error::Invalid(e.to_string()))?
            .as_bytes());
        let var_names: Vec<String> = mission.vars.keys().cloned().collect();
        let vars = mission.vars.values().copied().collect();
        let mut runtime = Self {
            hash,
            tick_rate: tick_rate.max(1),
            steps: Vec::new(),
            ids: step_ids(&mission.steps),
            conds: Vec::new(),
            names: Vec::new(),
            var_names,
            objective_step: vec![NONE; mission.objectives.len()],
            state: Vec::new(),
            latches: Vec::new(),
            items: vec![ItemState::Absent; mission.items.len()],
            vars,
            last_event: Vec::new(),
            pending: Vec::new(),
            outcome: Outcome::Running,
            events: Vec::new(),
            mission,
        };
        let root = runtime.mission.steps.clone();
        runtime.compile_step(&root);
        runtime.state = vec![StepState::default(); runtime.steps.len()];
        runtime.latches = vec![false; runtime.conds.len()];
        runtime.last_event = vec![None; runtime.names.len()];
        Ok(runtime)
    }

    /// The mission at `stage` ([`Stage`]).
    pub fn with_stage(mission: Mission, tick_rate: u32, stage: &Stage) -> Result<Self, Error> {
        let mut runtime = Self::new(mission, tick_rate)?;
        if stage.mission != runtime.mission.id {
            return Err(Error::State(format!(
                "a stage of {} loaded into {}",
                stage.mission, runtime.mission.id
            )));
        }
        for (id, ended) in &stage.ended {
            let id = runtime.mission.renamed.get(id).unwrap_or(id);
            if let Some(n) = runtime.ids.iter().position(|i| i == id) {
                runtime.state[n].phase = match ended {
                    Ended::Succeeded => Phase::Succeeded,
                    Ended::Failed => Phase::Failed,
                    Ended::Cancelled => Phase::Cancelled,
                };
            }
        }
        for (n, name) in runtime.var_names.iter().enumerate() {
            if let Some(&v) = stage.vars.get(name) {
                runtime.vars[n] = v;
            }
        }
        for (n, item) in runtime.mission.items.iter().enumerate() {
            if stage.picked.contains(&item.id) {
                runtime.items[n] = ItemState::Picked { tick: 0 };
            } else if stage.present.contains(&item.id) {
                runtime.items[n] = ItemState::Present;
            }
        }
        runtime.outcome = stage.outcome;
        Ok(runtime)
    }

    /// Replaces the record with `mission`, keeping the stage (hot reload).
    pub fn reload(&mut self, mission: Mission) -> Result<(), Error> {
        *self = Self::with_stage(mission, self.tick_rate, &self.stage())?;
        Ok(())
    }

    fn compile_step(&mut self, step: &Step) -> u32 {
        let index = self.steps.len() as u32;
        self.steps.push(StepNode {
            kind: Kind::Do,
            children: Vec::new(),
            objective: None,
            optional: step.optional,
            actions: Vec::new(),
            then: self.compile_actions(&step.then),
            on_fail: self.compile_actions(&step.on_fail),
            on_cancel: self.compile_actions(&step.on_cancel),
        });
        if let Some(o) = &step.objective {
            let o = self.mission.objectives.iter().position(|x| &x.id == o);
            let o = o.expect("validated");
            self.objective_step[o] = index;
            self.steps[index as usize].objective = Some(o as u16);
        }
        let kind = if step.until.is_some() || step.fail_if.is_some() {
            Kind::Wait {
                until: step
                    .until
                    .as_ref()
                    .map_or(NONE, |c| self.compile_condition(c)),
                fail_if: step
                    .fail_if
                    .as_ref()
                    .map_or(NONE, |c| self.compile_condition(c)),
            }
        } else if step.seq.is_some() {
            Kind::Seq
        } else if step.all.is_some() {
            Kind::All
        } else if step.race.is_some() {
            Kind::Race
        } else if step.branch.is_some() {
            Kind::Branch
        } else {
            let actions = self.compile_actions(step.actions.as_deref().unwrap_or_default());
            self.steps[index as usize].actions = actions;
            Kind::Do
        };
        let kids: Vec<u32> = children(step)
            .into_iter()
            .map(|c| self.compile_step(c))
            .collect();
        let node = &mut self.steps[index as usize];
        node.kind = kind;
        node.children = kids;
        index
    }

    fn compile_actions(&self, actions: &[Action]) -> Vec<Act> {
        actions
            .iter()
            .map(|a| match a {
                Action::Spawn(id) => Act::Spawn(self.item(id)),
                Action::Set(name, v) => Act::Set(self.var(name), *v),
                Action::Add(name, d) => Act::Add(self.var(name), *d),
                Action::Emit(tag) => Act::Emit(tag.clone()),
                Action::End(e) => Act::End(*e),
            })
            .collect()
    }

    fn item(&self, id: &str) -> u16 {
        let i = self.mission.items.iter().position(|i| i.id == id);
        i.expect("validated") as u16
    }

    fn var(&self, name: &str) -> u16 {
        self.var_names
            .iter()
            .position(|n| n == name)
            .expect("validated") as u16
    }

    fn compile_condition(&mut self, condition: &Condition) -> u32 {
        let index = self.conds.len() as u32;
        self.conds.push(CondNode {
            kind: CondKind::Never,
            children: Vec::new(),
        });
        let (kind, children) = match condition {
            Condition::Inside(id) => {
                let p = self.mission.places.iter().position(|p| &p.id == id);
                (CondKind::Inside(p.expect("validated") as u16), Vec::new())
            }
            Condition::Near(id, metres) => {
                let target = match self.mission.places.iter().position(|p| &p.id == id) {
                    Some(p) => Target::Place(p as u16),
                    None => Target::Item(self.item(id)),
                };
                (CondKind::Near(target, metres * metres), Vec::new())
            }
            Condition::Holds(id) => (CondKind::Holds(self.item(id)), Vec::new()),
            Condition::After(seconds) => {
                let ticks = (seconds * f64::from(self.tick_rate)).ceil() as u64;
                (CondKind::After(ticks), Vec::new())
            }
            Condition::Var(name, compare) => (CondKind::Var(self.var(name), *compare), Vec::new()),
            Condition::Event(name) => {
                let e = match self.names.iter().position(|n| n == name) {
                    Some(e) => e,
                    None => {
                        self.names.push(name.clone());
                        self.names.len() - 1
                    }
                };
                (CondKind::Event(e as u16), Vec::new())
            }
            Condition::AllOf(cs) => (
                CondKind::AllOf,
                cs.iter().map(|c| self.compile_condition(c)).collect(),
            ),
            Condition::AnyOf(cs) => (
                CondKind::AnyOf,
                cs.iter().map(|c| self.compile_condition(c)).collect(),
            ),
            Condition::Not(c) => (CondKind::Not, vec![self.compile_condition(c)]),
            Condition::Never => (CondKind::Never, Vec::new()),
        };
        self.conds[index as usize] = CondNode { kind, children };
        index
    }

    /// The record.
    pub fn mission(&self) -> &Mission {
        &self.mission
    }

    /// How it stands.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// What the last tick changed.
    pub fn events(&self) -> &[MissionEvent] {
        &self.events
    }

    /// The objectives and their status (their steps'), in the record's order.
    pub fn objectives(&self) -> impl Iterator<Item = (&Objective, Status)> {
        self.mission
            .objectives
            .iter()
            .zip(&self.objective_step)
            .map(|(o, &n)| {
                let status = match self.state.get(n as usize).map(|s| s.phase) {
                    None | Some(Phase::Idle) => Status::Pending,
                    Some(Phase::Running) => Status::Active,
                    Some(Phase::Succeeded) => Status::Completed,
                    Some(Phase::Failed) => Status::Failed,
                    Some(Phase::Cancelled) => Status::Missed,
                };
                (o, status)
            })
    }

    /// The items and their state, in the record's order.
    pub fn items(&self) -> impl Iterator<Item = (&Item, ItemState)> {
        self.mission.items.iter().zip(self.items.iter().copied())
    }

    /// The counters and their values, by name.
    pub fn vars(&self) -> impl Iterator<Item = (&str, i64)> {
        self.var_names
            .iter()
            .map(String::as_str)
            .zip(self.vars.iter().copied())
    }

    /// How the step `id` stands; None when no step is `id`.
    pub fn step(&self, id: &str) -> Option<StepStatus> {
        let n = self.ids.iter().position(|i| i == id)?;
        Some(match self.state[n].phase {
            Phase::Idle => StepStatus::Idle,
            Phase::Running => StepStatus::Running,
            Phase::Succeeded => StepStatus::Succeeded,
            Phase::Failed => StepStatus::Failed,
            Phase::Cancelled => StepStatus::Cancelled,
        })
    }

    /// Where the waits under way point, in the steps' order: the places to enter or come near
    /// (their middles) and the items to pick up or come near (where they lie, once spawned). A
    /// walker heads for the first; an interface marks them. What a wait's `fail_if` names is not
    /// a target.
    pub fn targets(&self) -> Vec<DVec3> {
        let mut out = Vec::new();
        for (n, node) in self.steps.iter().enumerate() {
            if let Kind::Wait { until, .. } = node.kind
                && until != NONE
                && self.state[n].phase == Phase::Running
            {
                self.condition_targets(until, &mut out);
            }
        }
        out
    }

    fn condition_targets(&self, c: u32, out: &mut Vec<DVec3>) {
        let node = &self.conds[c as usize];
        if self.latches[c as usize] {
            return;
        }
        let item_at = |i: u16| {
            (self.items[usize::from(i)] == ItemState::Present)
                .then(|| DVec3::from(self.mission.items[usize::from(i)].at))
        };
        match node.kind {
            CondKind::Inside(p) | CondKind::Near(Target::Place(p), _) => {
                out.push(DVec3::from(
                    self.mission.places[usize::from(p)].shape.centre(),
                ));
            }
            CondKind::Holds(i) | CondKind::Near(Target::Item(i), _) => out.extend(item_at(i)),
            CondKind::AllOf | CondKind::AnyOf => {
                for &k in &node.children {
                    self.condition_targets(k, out);
                }
            }
            _ => {}
        }
    }

    /// An event the game sends by name (a kill, a door opened): it dates from the next tick.
    /// Names no condition of the mission uses are ignored.
    pub fn notify(&mut self, name: &str) {
        if let Some(e) = self.names.iter().position(|n| n == name) {
            self.pending.push(e as u16);
        }
    }

    /// Tick `tick`, the player's feet at `player` (metres, y up).
    pub fn tick(&mut self, tick: u64, player: DVec3) {
        self.events.clear();
        for e in std::mem::take(&mut self.pending) {
            self.last_event[usize::from(e)] = Some(tick);
        }
        if self.outcome != Outcome::Running {
            return;
        }
        let p = player.to_array();
        for (n, item) in self.mission.items.iter().enumerate() {
            if self.items[n] == ItemState::Present
                && distance2(p, item.at) <= item.reach * item.reach
            {
                self.items[n] = ItemState::Picked { tick };
                self.events.push(MissionEvent::ItemPicked(item.id.clone()));
            }
        }
        if self.phase(0) == Phase::Idle {
            self.start(0, tick);
        }
        if self.phase(0) == Phase::Running {
            self.update(0, tick, p);
        }
        if self.outcome == Outcome::Running {
            match self.phase(0) {
                Phase::Succeeded => self.finish_mission(End::Succeeded),
                Phase::Failed | Phase::Cancelled => self.finish_mission(End::Failed),
                _ => {}
            }
        }
        if self.outcome != Outcome::Running {
            self.cancel(0);
        }
    }

    fn finish_mission(&mut self, end: End) {
        if self.outcome == Outcome::Running {
            self.outcome = match end {
                End::Succeeded => Outcome::Succeeded,
                End::Failed => Outcome::Failed,
            };
            self.events.push(match end {
                End::Succeeded => MissionEvent::Succeeded,
                End::Failed => MissionEvent::Failed,
            });
        }
    }

    fn phase(&self, n: u32) -> Phase {
        self.state[n as usize].phase
    }

    fn running(&self) -> bool {
        self.outcome == Outcome::Running
    }

    /// Whether step `c`, for its parent, is past: ended, or a branch whose step runs on in the
    /// background (a branch succeeds at once).
    fn past(&self, c: u32) -> bool {
        let phase = self.phase(c);
        phase.ended() || (self.steps[c as usize].kind == Kind::Branch && phase == Phase::Running)
    }

    /// Whether step `c`, for its parent, succeeded: a branch running on counts.
    fn succeeded(&self, c: u32) -> bool {
        let phase = self.phase(c);
        phase == Phase::Succeeded
            || (self.steps[c as usize].kind == Kind::Branch && phase == Phase::Running)
    }

    /// Begins step `n`: a `do` runs its actions and succeeds; `all`, `race` and `branch` begin
    /// their steps with it.
    fn start(&mut self, n: u32, tick: u64) {
        self.state[n as usize] = StepState {
            phase: Phase::Running,
            since: tick,
            cursor: 0,
        };
        if let Some(o) = self.steps[n as usize].objective {
            let id = self.mission.objectives[usize::from(o)].id.clone();
            self.events.push(MissionEvent::ObjectiveStarted(id));
        }
        match self.steps[n as usize].kind {
            Kind::Wait { until, fail_if } => {
                for c in [until, fail_if] {
                    if c != NONE {
                        self.reset_latches(c);
                    }
                }
            }
            Kind::Do => {
                let actions = self.steps[n as usize].actions.clone();
                self.apply(&actions);
                if self.phase(n) == Phase::Running {
                    self.end(n, Phase::Succeeded);
                }
            }
            Kind::Seq => {}
            Kind::All | Kind::Race | Kind::Branch => {
                for c in self.steps[n as usize].children.clone() {
                    if !self.running() || self.phase(n) != Phase::Running {
                        break;
                    }
                    if self.phase(c) == Phase::Idle {
                        self.start(c, tick);
                    }
                }
            }
        }
    }

    /// Updates running step `n` and its running steps.
    fn update(&mut self, n: u32, tick: u64, p: [f64; 3]) {
        let children = self.steps[n as usize].children.clone();
        match self.steps[n as usize].kind {
            Kind::Wait { until, fail_if } => {
                let since = self.state[n as usize].since;
                // A tie goes to failure (D-073).
                if fail_if != NONE && self.holds(fail_if, true, since, tick, p) {
                    self.end(n, Phase::Failed);
                } else if until != NONE && self.holds(until, true, since, tick, p) {
                    self.end(n, Phase::Succeeded);
                }
            }
            Kind::Do => {}
            Kind::Seq => {
                // The steps before the one in progress still running are branches.
                let cursor = self.state[n as usize].cursor as usize;
                for &c in &children[..cursor.min(children.len())] {
                    if self.phase(c) == Phase::Running && self.running() {
                        self.update(c, tick, p);
                    }
                }
                loop {
                    if !self.running() || self.phase(n) != Phase::Running {
                        return;
                    }
                    let cursor = self.state[n as usize].cursor as usize;
                    let Some(&c) = children.get(cursor) else {
                        self.end(n, Phase::Succeeded);
                        return;
                    };
                    if self.phase(c) == Phase::Idle {
                        self.start(c, tick);
                    }
                    if self.phase(c) == Phase::Running {
                        self.update(c, tick, p);
                    }
                    if !self.past(c) {
                        return;
                    }
                    if self.phase(c) == Phase::Failed && !self.steps[c as usize].optional {
                        self.end(n, Phase::Failed);
                        return;
                    }
                    self.state[n as usize].cursor += 1;
                }
            }
            Kind::All => {
                for &c in &children {
                    if self.phase(c) == Phase::Running && self.running() {
                        self.update(c, tick, p);
                    }
                }
                if !self.running() || self.phase(n) != Phase::Running {
                    return;
                }
                let required = children
                    .iter()
                    .filter(|&&c| !self.steps[c as usize].optional);
                if required.clone().any(|&c| self.phase(c) == Phase::Failed) {
                    self.end(n, Phase::Failed);
                } else if required.clone().all(|&c| self.succeeded(c)) {
                    self.end(n, Phase::Succeeded);
                }
            }
            Kind::Race => {
                for &c in &children {
                    if !self.running() || self.phase(n) != Phase::Running {
                        return;
                    }
                    if self.phase(c) == Phase::Running {
                        self.update(c, tick, p);
                    }
                    if self.past(c) {
                        // The first to end decides it, failed or not (Verse's rule).
                        let phase = if self.succeeded(c) {
                            Phase::Succeeded
                        } else {
                            Phase::Failed
                        };
                        self.end(n, phase);
                        return;
                    }
                }
            }
            Kind::Branch => {
                let c = children[0];
                if self.phase(c) == Phase::Running {
                    self.update(c, tick, p);
                }
                if self.phase(c).ended() && self.phase(n) == Phase::Running {
                    self.end(n, Phase::Succeeded);
                }
            }
        }
    }

    /// Step `n` ended as `phase` (succeeded or failed): what still runs under it (branches, a
    /// race's others, an `all`'s optional steps) is cancelled, its objective follows, and its
    /// `then` or `on_fail` runs.
    fn end(&mut self, n: u32, phase: Phase) {
        self.state[n as usize].phase = phase;
        for c in self.steps[n as usize].children.clone() {
            self.cancel(c);
        }
        if let Some(o) = self.steps[n as usize].objective {
            let id = self.mission.objectives[usize::from(o)].id.clone();
            self.events.push(if phase == Phase::Succeeded {
                MissionEvent::ObjectiveCompleted(id)
            } else {
                MissionEvent::ObjectiveFailed(id)
            });
        }
        let actions = if phase == Phase::Succeeded {
            self.steps[n as usize].then.clone()
        } else {
            self.steps[n as usize].on_fail.clone()
        };
        self.apply(&actions);
    }

    /// Cancels the running steps from `n` down, in the tree's order: their objectives are missed
    /// and their `on_cancel` runs.
    fn cancel(&mut self, n: u32) {
        if self.phase(n) == Phase::Running {
            self.state[n as usize].phase = Phase::Cancelled;
            if let Some(o) = self.steps[n as usize].objective {
                let id = self.mission.objectives[usize::from(o)].id.clone();
                self.events.push(MissionEvent::ObjectiveMissed(id));
            }
            let actions = self.steps[n as usize].on_cancel.clone();
            self.apply(&actions);
        }
        for c in self.steps[n as usize].children.clone() {
            self.cancel(c);
        }
    }

    /// Applies `actions` in order.
    fn apply(&mut self, actions: &[Act]) {
        for a in actions {
            match a {
                Act::Spawn(i) => {
                    let i = usize::from(*i);
                    if self.items[i] == ItemState::Absent {
                        self.items[i] = ItemState::Present;
                        let id = self.mission.items[i].id.clone();
                        self.events.push(MissionEvent::ItemSpawned(id));
                    }
                }
                Act::Set(v, value) => self.vars[usize::from(*v)] = *value,
                Act::Add(v, d) => {
                    let v = &mut self.vars[usize::from(*v)];
                    *v = v.saturating_add(*d);
                }
                Act::Emit(tag) => self.events.push(MissionEvent::Emitted(tag.clone())),
                Act::End(end) => self.finish_mission(*end),
            }
        }
    }

    fn reset_latches(&mut self, c: u32) {
        self.latches[c as usize] = false;
        for k in self.conds[c as usize].children.clone() {
            self.reset_latches(k);
        }
    }

    /// Whether condition `c` of a step begun at `since` holds at `tick`. With `latch`, a node
    /// that held stays held for the step; under a `Not` it is tested as it is now.
    fn holds(&mut self, c: u32, latch: bool, since: u64, tick: u64, p: [f64; 3]) -> bool {
        if latch && self.latches[c as usize] {
            return true;
        }
        let node = self.conds[c as usize].clone();
        let now = match node.kind {
            CondKind::Inside(place) => self.mission.places[usize::from(place)].shape.contains(p),
            CondKind::Near(Target::Place(place), d2) => {
                let centre = self.mission.places[usize::from(place)].shape.centre();
                distance2(p, centre) <= d2
            }
            CondKind::Near(Target::Item(i), d2) => {
                self.items[usize::from(i)] == ItemState::Present
                    && distance2(p, self.mission.items[usize::from(i)].at) <= d2
            }
            CondKind::Holds(i) => matches!(self.items[usize::from(i)], ItemState::Picked { .. }),
            CondKind::After(ticks) => tick.saturating_sub(since) >= ticks,
            CondKind::Var(v, compare) => compare.holds(self.vars[usize::from(v)]),
            CondKind::Event(e) => self.last_event[usize::from(e)].is_some_and(|at| at >= since),
            CondKind::AllOf => {
                // Every part asked, so each latches when it holds.
                let mut all = true;
                for &k in &node.children {
                    all &= self.holds(k, latch, since, tick, p);
                }
                all
            }
            CondKind::AnyOf => {
                let mut any = false;
                for &k in &node.children {
                    any |= self.holds(k, latch, since, tick, p);
                }
                any
            }
            CondKind::Not => !self.holds(node.children[0], false, since, tick, p),
            CondKind::Never => false,
        };
        if now && latch {
            self.latches[c as usize] = true;
        }
        now
    }

    /// The progress by id, for a save game or a reload ([`Stage`]).
    pub fn stage(&self) -> Stage {
        let ended = self
            .ids
            .iter()
            .zip(&self.state)
            .filter_map(|(id, s)| {
                let e = match s.phase {
                    Phase::Succeeded => Ended::Succeeded,
                    Phase::Failed => Ended::Failed,
                    Phase::Cancelled => Ended::Cancelled,
                    _ => return None,
                };
                Some((id.clone(), e))
            })
            .collect();
        let named = |state: fn(ItemState) -> bool| {
            self.items()
                .filter(|(_, s)| state(*s))
                .map(|(i, _)| i.id.clone())
                .collect()
        };
        Stage {
            mission: self.mission.id.clone(),
            ended,
            vars: self.vars().map(|(k, v)| (k.to_owned(), v)).collect(),
            present: named(|s| s == ItemState::Present),
            picked: named(|s| matches!(s, ItemState::Picked { .. })),
            outcome: self.outcome,
        }
    }

    /// The exact state appended to `out` (a length first): it loads back into this record only
    /// ([`Runtime::restore`]). The events sent since the last tick are not part of it.
    pub fn save(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0; 4]);
        self.write(out);
        let len = (out.len() - start - 4) as u32;
        out[start..start + 4].copy_from_slice(&len.to_le_bytes());
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.hash.to_le_bytes());
        out.push(self.outcome as u8);
        for s in &self.state {
            out.push(s.phase as u8);
            out.extend_from_slice(&s.since.to_le_bytes());
            out.extend_from_slice(&s.cursor.to_le_bytes());
        }
        out.extend(self.latches.iter().map(|&l| u8::from(l)));
        for item in &self.items {
            let (code, tick) = match *item {
                ItemState::Absent => (0u8, 0),
                ItemState::Present => (1, 0),
                ItemState::Picked { tick } => (2, tick),
            };
            out.push(code);
            out.extend_from_slice(&tick.to_le_bytes());
        }
        for v in &self.vars {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for e in &self.last_event {
            out.extend_from_slice(&e.unwrap_or(u64::MAX).to_le_bytes());
        }
    }

    /// Loads a state [`Runtime::save`] wrote from the start of `bytes`; returns the bytes it
    /// took. A state of another record (another hash) is refused.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<usize, Error> {
        let bad = |what: &str| Error::State(format!("{}: {what}", self.mission.id));
        let mut r = Reader { bytes, at: 0 };
        let len = r.u32().ok_or_else(|| bad("no length"))? as usize;
        let mut r = Reader {
            bytes: bytes.get(4..4 + len).ok_or_else(|| bad("too short"))?,
            at: 0,
        };
        if r.u64() != Some(self.hash) {
            return Err(bad("a state of another version of the record"));
        }
        let outcome = r
            .u8()
            .and_then(Outcome::of)
            .ok_or_else(|| bad("the outcome"))?;
        let mut state = self.state.clone();
        for s in &mut state {
            *s = StepState {
                phase: r.u8().and_then(Phase::of).ok_or_else(|| bad("a step"))?,
                since: r.u64().ok_or_else(|| bad("a step"))?,
                cursor: r.u32().ok_or_else(|| bad("a step"))?,
            };
        }
        let mut latches = self.latches.clone();
        for l in &mut latches {
            *l = r.u8().ok_or_else(|| bad("a condition"))? != 0;
        }
        let mut items = self.items.clone();
        for item in &mut items {
            let code = r.u8().ok_or_else(|| bad("an item"))?;
            let tick = r.u64().ok_or_else(|| bad("an item"))?;
            *item = match code {
                0 => ItemState::Absent,
                1 => ItemState::Present,
                2 => ItemState::Picked { tick },
                _ => return Err(bad("an item")),
            };
        }
        let mut vars = self.vars.clone();
        for v in &mut vars {
            *v = r.i64().ok_or_else(|| bad("a counter"))?;
        }
        let mut last_event = self.last_event.clone();
        for e in &mut last_event {
            let tick = r.u64().ok_or_else(|| bad("an event"))?;
            *e = (tick != u64::MAX).then_some(tick);
        }
        if r.at != len {
            return Err(bad("bytes left over"));
        }
        self.outcome = outcome;
        self.state = state;
        self.latches = latches;
        self.items = items;
        self.vars = vars;
        self.last_event = last_event;
        self.pending.clear();
        self.events.clear();
        Ok(4 + len)
    }

    /// The state's digest (D-016): equal states, equal digests.
    pub fn digest(&self) -> u64 {
        let mut bytes = Vec::new();
        self.write(&mut bytes);
        fnv(&bytes)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let out = self.bytes.get(self.at..self.at + N)?.try_into().ok()?;
        self.at += N;
        Some(out)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|b| b[0])
    }

    fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take().map(u64::from_le_bytes)
    }

    fn i64(&mut self) -> Option<i64> {
        self.take().map(i64::from_le_bytes)
    }
}

fn distance2(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| (a[k] - b[k]) * (a[k] - b[k])).sum()
}

/// FNV-1a, 64 bits.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

//! A mission evaluated at the fixed tick (D-072): its steps compiled into a flat tree in their
//! written order, each node's state, the items and the objectives. The state is saved exactly
//! (a snapshot: rollback, replays, digests) or by stage (what is done, by id: save games and hot
//! reload, which load into a changed record).
//!
//! Each tick: the events the game sent are dated, the items within the player's reach are picked
//! up, then the tree is updated from its root, a node's children in their written order. A step
//! that ends at once lets the next start in the same tick, so a chain of spawns and instant waits
//! takes no time.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::schema::{Item, Mission, Objective, Step, Trigger};

/// A node of the steps' tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Its trigger's root, an index into the triggers.
    Wait(u32),
    Sequence,
    All,
    Race,
    Branch,
    /// The objective's index.
    Objective(u16),
    /// The item's index.
    Spawn(u16),
    Succeed,
    Fail,
}

/// A node of a wait's trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TriggerKind {
    /// The place's index.
    Entered(u16),
    /// The item's index.
    PickedUp(u16),
    /// Ticks since the wait began.
    After(u64),
    /// The event's index among the names the mission uses.
    Event(u16),
    AllOf,
    AnyOf,
}

#[derive(Clone, Debug)]
struct Node<K> {
    kind: K,
    children: Vec<u32>,
}

/// Where a step stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle = 0,
    Running = 1,
    Done = 2,
    Cancelled = 3,
}

impl Phase {
    fn of(byte: u8) -> Option<Self> {
        [Self::Idle, Self::Running, Self::Done, Self::Cancelled]
            .get(usize::from(byte))
            .copied()
    }
}

/// A step's state: its phase, the tick it began, and a sequence's step in progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StepState {
    phase: Phase,
    since: u64,
    cursor: u32,
}

/// An objective's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    /// Its step has not begun.
    Pending,
    /// Shown: its step runs.
    Active,
    /// Its step is done.
    Completed,
    /// Its step was cancelled (a race lost, the mission ended).
    Cancelled,
}

impl Status {
    fn of(byte: u8) -> Option<Self> {
        [
            Self::Pending,
            Self::Active,
            Self::Completed,
            Self::Cancelled,
        ]
        .get(usize::from(byte))
        .copied()
    }
}

/// How the mission stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Still going.
    #[default]
    Running,
    /// Won: a [`Step::Succeed`], or every step done.
    Succeeded,
    /// Lost: a [`Step::Fail`].
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
    /// An objective was cancelled.
    ObjectiveCancelled(String),
    /// An item appeared at its place.
    ItemSpawned(String),
    /// The player picked an item up.
    ItemPicked(String),
    /// The mission is won.
    Succeeded,
    /// The mission is lost.
    Failed,
}

/// A mission's progress as a save game keeps it: what is done, by id. It loads into the
/// mission's record even after the record changed (hot reload, a newer version): the objectives
/// done are skipped, the items picked stay picked, and the step that was in progress starts
/// over (its timers too).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    /// The mission's id.
    pub mission: String,
    /// The objectives completed.
    pub completed: Vec<String>,
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
    steps: Vec<Node<Kind>>,
    triggers: Vec<Node<TriggerKind>>,
    /// The event names the triggers use.
    names: Vec<String>,
    state: Vec<StepState>,
    /// Per trigger node: fired since its wait began.
    latches: Vec<bool>,
    items: Vec<ItemState>,
    objectives: Vec<Status>,
    /// Per event name, the last tick it was sent.
    last_event: Vec<Option<u64>>,
    /// The events sent since the last tick.
    pending: Vec<u16>,
    outcome: Outcome,
    /// What the last tick changed.
    events: Vec<MissionEvent>,
}

impl Runtime {
    /// The mission at its start, with `tick_rate` ticks a second (its waits' seconds become
    /// ticks, rounded up). The record must be valid ([`crate::schema::validate`]).
    pub fn new(mission: Mission, tick_rate: u32) -> Result<Self, Error> {
        crate::schema::validate(&mission)?;
        let hash = fnv(ron::to_string(&mission)
            .map_err(|e| Error::Invalid(e.to_string()))?
            .as_bytes());
        let mut runtime = Self {
            hash,
            tick_rate: tick_rate.max(1),
            steps: Vec::new(),
            triggers: Vec::new(),
            names: Vec::new(),
            state: Vec::new(),
            latches: Vec::new(),
            items: vec![ItemState::Absent; mission.items.len()],
            objectives: vec![Status::Pending; mission.objectives.len()],
            last_event: Vec::new(),
            pending: Vec::new(),
            outcome: Outcome::Running,
            events: Vec::new(),
            mission,
        };
        let root = runtime.mission.steps.clone();
        runtime.compile_step(&root);
        runtime.state = vec![StepState::default(); runtime.steps.len()];
        runtime.latches = vec![false; runtime.triggers.len()];
        runtime.last_event = vec![None; runtime.names.len()];
        Ok(runtime)
    }

    /// The mission at `stage`: the objectives done skipped, the items picked up kept.
    pub fn with_stage(mission: Mission, tick_rate: u32, stage: &Stage) -> Result<Self, Error> {
        let mut runtime = Self::new(mission, tick_rate)?;
        if stage.mission != runtime.mission.id {
            return Err(Error::State(format!(
                "a stage of {} loaded into {}",
                stage.mission, runtime.mission.id
            )));
        }
        for (n, objective) in runtime.mission.objectives.iter().enumerate() {
            if stage.completed.contains(&objective.id) {
                runtime.objectives[n] = Status::Completed;
            }
        }
        for (n, item) in runtime.mission.items.iter().enumerate() {
            if stage.picked.contains(&item.id) {
                runtime.items[n] = ItemState::Picked { tick: 0 };
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
        self.steps.push(Node {
            kind: Kind::Succeed,
            children: Vec::new(),
        });
        let (kind, children) = match step {
            Step::Wait(t) => (Kind::Wait(self.compile_trigger(t)), Vec::new()),
            Step::Sequence(s) => (Kind::Sequence, self.compile_steps(s)),
            Step::All(s) => (Kind::All, self.compile_steps(s)),
            Step::Race(s) => (Kind::Race, self.compile_steps(s)),
            Step::Branch(s) => (Kind::Branch, vec![self.compile_step(s)]),
            Step::Objective(id, s) => {
                let o = self.mission.objectives.iter().position(|o| &o.id == id);
                let o = o.expect("validated") as u16;
                (Kind::Objective(o), vec![self.compile_step(s)])
            }
            Step::Spawn(id) => {
                let i = self.mission.items.iter().position(|i| &i.id == id);
                (Kind::Spawn(i.expect("validated") as u16), Vec::new())
            }
            Step::Succeed => (Kind::Succeed, Vec::new()),
            Step::Fail => (Kind::Fail, Vec::new()),
        };
        self.steps[index as usize] = Node { kind, children };
        index
    }

    fn compile_steps(&mut self, steps: &[Step]) -> Vec<u32> {
        steps.iter().map(|s| self.compile_step(s)).collect()
    }

    fn compile_trigger(&mut self, trigger: &Trigger) -> u32 {
        let index = self.triggers.len() as u32;
        self.triggers.push(Node {
            kind: TriggerKind::AllOf,
            children: Vec::new(),
        });
        let (kind, children) = match trigger {
            Trigger::Entered(id) => {
                let p = self.mission.places.iter().position(|p| &p.id == id);
                (
                    TriggerKind::Entered(p.expect("validated") as u16),
                    Vec::new(),
                )
            }
            Trigger::PickedUp(id) => {
                let i = self.mission.items.iter().position(|i| &i.id == id);
                (
                    TriggerKind::PickedUp(i.expect("validated") as u16),
                    Vec::new(),
                )
            }
            Trigger::After(seconds) => {
                let ticks = (seconds * f64::from(self.tick_rate)).ceil() as u64;
                (TriggerKind::After(ticks), Vec::new())
            }
            Trigger::Event(name) => {
                let e = match self.names.iter().position(|n| n == name) {
                    Some(e) => e,
                    None => {
                        self.names.push(name.clone());
                        self.names.len() - 1
                    }
                };
                (TriggerKind::Event(e as u16), Vec::new())
            }
            Trigger::AllOf(ts) => (
                TriggerKind::AllOf,
                ts.iter().map(|t| self.compile_trigger(t)).collect(),
            ),
            Trigger::AnyOf(ts) => (
                TriggerKind::AnyOf,
                ts.iter().map(|t| self.compile_trigger(t)).collect(),
            ),
        };
        self.triggers[index as usize] = Node { kind, children };
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

    /// The objectives and their status, in the record's order.
    pub fn objectives(&self) -> impl Iterator<Item = (&Objective, Status)> {
        self.mission
            .objectives
            .iter()
            .zip(self.objectives.iter().copied())
    }

    /// The items and their state, in the record's order.
    pub fn items(&self) -> impl Iterator<Item = (&Item, ItemState)> {
        self.mission.items.iter().zip(self.items.iter().copied())
    }

    /// Where the waits under way point, in the steps' order: the places to enter (their
    /// middles) and the items to pick up (where they lie, once spawned). A walker heads for the
    /// first; an interface marks them.
    pub fn targets(&self) -> Vec<DVec3> {
        let mut out = Vec::new();
        for (n, node) in self.steps.iter().enumerate() {
            if let Kind::Wait(t) = node.kind
                && self.state[n].phase == Phase::Running
            {
                self.trigger_targets(t, &mut out);
            }
        }
        out
    }

    fn trigger_targets(&self, t: u32, out: &mut Vec<DVec3>) {
        let node = &self.triggers[t as usize];
        if self.latches[t as usize] {
            return;
        }
        match node.kind {
            TriggerKind::Entered(p) => {
                out.push(DVec3::from(
                    self.mission.places[usize::from(p)].shape.centre(),
                ));
            }
            TriggerKind::PickedUp(i) if self.items[usize::from(i)] == ItemState::Present => {
                out.push(DVec3::from(self.mission.items[usize::from(i)].at));
            }
            TriggerKind::AllOf | TriggerKind::AnyOf => {
                for &c in &node.children {
                    self.trigger_targets(c, out);
                }
            }
            _ => {}
        }
    }

    /// An event the game sends by name (a kill, a door opened): it dates from the next tick.
    /// Names no trigger of the mission uses are ignored.
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
        if self.state[0].phase == Phase::Idle {
            self.start(0, tick);
        }
        if self.state[0].phase == Phase::Running {
            self.update(0, tick, p);
        }
        if self.outcome == Outcome::Running && self.state[0].phase == Phase::Done {
            self.outcome = Outcome::Succeeded;
            self.events.push(MissionEvent::Succeeded);
        }
        if self.outcome != Outcome::Running {
            self.cancel(0);
        }
    }

    fn kind(&self, n: u32) -> Kind {
        self.steps[n as usize].kind
    }

    fn phase(&self, n: u32) -> Phase {
        self.state[n as usize].phase
    }

    /// Begins step `n`: a spawn, a success or a failure happens at once, an objective already
    /// completed (a stage loaded) is done at once; the children of the steps that run them
    /// together begin with it.
    fn start(&mut self, n: u32, tick: u64) {
        self.state[n as usize] = StepState {
            phase: Phase::Running,
            since: tick,
            cursor: 0,
        };
        match self.kind(n) {
            Kind::Wait(t) => self.reset_latches(t),
            Kind::Objective(o) => {
                let o = usize::from(o);
                if self.objectives[o] == Status::Completed {
                    self.state[n as usize].phase = Phase::Done;
                    return;
                }
                self.objectives[o] = Status::Active;
                let id = self.mission.objectives[o].id.clone();
                self.events.push(MissionEvent::ObjectiveStarted(id));
            }
            Kind::Spawn(i) => {
                let i = usize::from(i);
                if self.items[i] == ItemState::Absent {
                    self.items[i] = ItemState::Present;
                    let id = self.mission.items[i].id.clone();
                    self.events.push(MissionEvent::ItemSpawned(id));
                }
                self.state[n as usize].phase = Phase::Done;
                return;
            }
            Kind::Succeed | Kind::Fail => {
                if self.outcome == Outcome::Running {
                    let won = self.kind(n) == Kind::Succeed;
                    self.outcome = if won {
                        Outcome::Succeeded
                    } else {
                        Outcome::Failed
                    };
                    self.events.push(if won {
                        MissionEvent::Succeeded
                    } else {
                        MissionEvent::Failed
                    });
                }
                self.state[n as usize].phase = Phase::Done;
                return;
            }
            Kind::Sequence => {}
            Kind::All | Kind::Race | Kind::Branch => {}
        }
        if matches!(
            self.kind(n),
            Kind::All | Kind::Race | Kind::Branch | Kind::Objective(_)
        ) {
            for c in self.steps[n as usize].children.clone() {
                if self.outcome != Outcome::Running {
                    break;
                }
                self.start(c, tick);
            }
        }
    }

    /// Updates running step `n` and its running children.
    fn update(&mut self, n: u32, tick: u64, p: [f64; 3]) {
        let children = self.steps[n as usize].children.clone();
        match self.kind(n) {
            Kind::Wait(t) => {
                let since = self.state[n as usize].since;
                if self.fires(t, since, tick, p) {
                    self.finish(n);
                }
            }
            Kind::Sequence => {
                // The steps before the one in progress still running are branches.
                let cursor = self.state[n as usize].cursor as usize;
                for &c in &children[..cursor.min(children.len())] {
                    if self.phase(c) == Phase::Running && self.outcome == Outcome::Running {
                        self.update(c, tick, p);
                    }
                }
                loop {
                    if self.outcome != Outcome::Running {
                        return;
                    }
                    let cursor = self.state[n as usize].cursor as usize;
                    let Some(&c) = children.get(cursor) else {
                        self.finish(n);
                        return;
                    };
                    if self.phase(c) == Phase::Idle {
                        self.start(c, tick);
                    }
                    if self.phase(c) == Phase::Running {
                        self.update(c, tick, p);
                    }
                    let past = self.phase(c) == Phase::Done
                        || (self.kind(c) == Kind::Branch && self.phase(c) == Phase::Running);
                    if !past {
                        return;
                    }
                    self.state[n as usize].cursor += 1;
                }
            }
            Kind::All => {
                for &c in &children {
                    if self.phase(c) == Phase::Running && self.outcome == Outcome::Running {
                        self.update(c, tick, p);
                    }
                }
                let all = children.iter().all(|&c| {
                    self.phase(c) == Phase::Done
                        || (self.kind(c) == Kind::Branch && self.phase(c) == Phase::Running)
                });
                if all && self.outcome == Outcome::Running {
                    self.finish(n);
                }
            }
            Kind::Race => {
                for &c in &children {
                    if self.outcome != Outcome::Running {
                        return;
                    }
                    if self.phase(c) == Phase::Running {
                        self.update(c, tick, p);
                    }
                    if self.phase(c) == Phase::Done {
                        self.finish(n);
                        return;
                    }
                }
            }
            Kind::Branch => {
                let c = children[0];
                if self.phase(c) == Phase::Running {
                    self.update(c, tick, p);
                }
                if self.phase(c) == Phase::Done {
                    self.finish(n);
                }
            }
            Kind::Objective(o) => {
                let c = children[0];
                if self.phase(c) == Phase::Running {
                    self.update(c, tick, p);
                }
                if self.phase(c) == Phase::Done && self.outcome == Outcome::Running {
                    let o = usize::from(o);
                    self.objectives[o] = Status::Completed;
                    let id = self.mission.objectives[o].id.clone();
                    self.events.push(MissionEvent::ObjectiveCompleted(id));
                    self.finish(n);
                }
            }
            Kind::Spawn(_) | Kind::Succeed | Kind::Fail => {}
        }
    }

    /// Step `n` done: what still runs under it (branches, a race's others) is cancelled.
    fn finish(&mut self, n: u32) {
        self.state[n as usize].phase = Phase::Done;
        for c in self.steps[n as usize].children.clone() {
            self.cancel(c);
        }
    }

    /// Cancels the running steps from `n` down; their objectives are cancelled.
    fn cancel(&mut self, n: u32) {
        if self.phase(n) == Phase::Running {
            self.state[n as usize].phase = Phase::Cancelled;
            if let Kind::Objective(o) = self.kind(n) {
                let o = usize::from(o);
                if self.objectives[o] == Status::Active {
                    self.objectives[o] = Status::Cancelled;
                    let id = self.mission.objectives[o].id.clone();
                    self.events.push(MissionEvent::ObjectiveCancelled(id));
                }
            }
        }
        for c in self.steps[n as usize].children.clone() {
            self.cancel(c);
        }
    }

    fn reset_latches(&mut self, t: u32) {
        self.latches[t as usize] = false;
        for c in self.triggers[t as usize].children.clone() {
            self.reset_latches(c);
        }
    }

    /// Whether trigger `t` of a wait begun at `since` has fired by `tick`; each part latches.
    fn fires(&mut self, t: u32, since: u64, tick: u64, p: [f64; 3]) -> bool {
        let node = self.triggers[t as usize].clone();
        let now = match node.kind {
            TriggerKind::Entered(place) => {
                self.mission.places[usize::from(place)].shape.contains(p)
            }
            TriggerKind::PickedUp(i) => {
                matches!(self.items[usize::from(i)], ItemState::Picked { .. })
            }
            TriggerKind::After(ticks) => tick.saturating_sub(since) >= ticks,
            TriggerKind::Event(e) => self.last_event[usize::from(e)].is_some_and(|at| at >= since),
            TriggerKind::AllOf => {
                // Every part asked, so each latches when it fires.
                let mut all = true;
                for &c in &node.children {
                    all &= self.fires(c, since, tick, p);
                }
                all
            }
            TriggerKind::AnyOf => {
                let mut any = false;
                for &c in &node.children {
                    any |= self.fires(c, since, tick, p);
                }
                any
            }
        };
        if now {
            self.latches[t as usize] = true;
        }
        self.latches[t as usize]
    }

    /// The progress by id, for a save game or a reload ([`Stage`]).
    pub fn stage(&self) -> Stage {
        let completed = self
            .objectives()
            .filter(|(_, s)| *s == Status::Completed)
            .map(|(o, _)| o.id.clone())
            .collect();
        let picked = self
            .items()
            .filter(|(_, s)| matches!(s, ItemState::Picked { .. }))
            .map(|(i, _)| i.id.clone())
            .collect();
        Stage {
            mission: self.mission.id.clone(),
            completed,
            picked,
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
        out.extend(self.objectives.iter().map(|&s| s as u8));
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
            *l = r.u8().ok_or_else(|| bad("a trigger"))? != 0;
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
        let mut objectives = self.objectives.clone();
        for o in &mut objectives {
            *o = r
                .u8()
                .and_then(Status::of)
                .ok_or_else(|| bad("an objective"))?;
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
        self.objectives = objectives;
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

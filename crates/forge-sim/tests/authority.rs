//! Authority across a two-worker boundary (issue #268, step 2's handoff; D-010's fourth proof
//! stage, in one process): 200 bots patrol back and forth across a boundary, keeping apart from
//! their neighbours, simulated by two workers that own the two sides, and the replication layer's
//! records match tick for tick, bit for bit, those of one worker holding everything.

use std::collections::BTreeMap;

use forge_core::SplitMix64;
use forge_sim::authority::{EntityId, Layer, Placed, Strips, Worker, WorkerId};
use glam::DVec3;

const TICK: f64 = 1.0 / 60.0;
/// How near a neighbour pushes a bot away, metres.
const REACH: f64 = 1.5;
/// Where the bots turn, metres either side of the boundary.
const TURN: f64 = 12.0;

/// A bot: where it stands, how it moves, and the side it walks to.
#[derive(Clone, Debug, PartialEq)]
struct Bot {
    position: DVec3,
    velocity: DVec3,
    goal: f64,
}

impl Placed for Bot {
    fn position(&self) -> DVec3 {
        self.position
    }
}

/// One tick of a bot, from its own state and its neighbours' at the barrier: towards its side at
/// its walk, pushed off whoever stands within reach, turning at the far end.
fn step(id: EntityId, bot: &Bot, world: &BTreeMap<EntityId, &Bot>) -> Bot {
    let mut push = DVec3::ZERO;
    for (&other, b) in world {
        if other == id {
            continue;
        }
        let d = bot.position - b.position;
        let l = d.length();
        if l < REACH && l > 1e-9 {
            push += d / l * (REACH - l);
        }
    }
    let walk = 1.2 + f64::from(id % 5) * 0.1;
    let along = (bot.goal - bot.position.x).signum() * walk;
    let velocity = DVec3::new(along, 0.0, 0.0) + push * 2.0;
    let position = bot.position + velocity * TICK;
    let goal = if (position.x - bot.goal).abs() < 0.5 {
        -bot.goal
    } else {
        bot.goal
    };
    Bot {
        position,
        velocity,
        goal,
    }
}

/// The bots at the start: along the strip, half walking each way.
fn bots(n: u32) -> Vec<(EntityId, Bot)> {
    let mut r = SplitMix64::new(7);
    (0..n)
        .map(|k| {
            let bot = Bot {
                position: DVec3::new(r.range_f64(-TURN, TURN), 0.0, r.range_f64(-10.0, 10.0)),
                velocity: DVec3::ZERO,
                goal: if k % 2 == 0 { TURN } else { -TURN },
            };
            (k * 3 + 1, bot)
        })
        .collect()
}

/// A worker's writes of a tick: each entity, the epoch it was granted, its new state.
type Writes = Vec<(EntityId, u32, Bot)>;

/// What can happen to the workers during a run.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Trouble {
    /// Nothing.
    None,
    /// Revokes reach the old owner up to two ticks late: it keeps writing at its old epoch.
    LateRevokes,
    /// Worker 1 goes away at tick 400 (worker 0 covers its region) and comes back at tick 600.
    WorkerAway,
    /// Worker 1 leaves for good at tick 400 and worker 2 joins at tick 600: the strips are cut
    /// again each time, balanced over where the bots stand.
    Membership,
}

/// What a run made: each tick's states, the handoffs and the refused writes.
struct Run {
    states: Vec<Vec<(EntityId, Bot)>>,
    handoffs: usize,
    /// Of them, those the worker away and back made, or the strips cut again.
    reassigned: usize,
    /// The bots each worker owned at the end.
    owned: BTreeMap<WorkerId, usize>,
    refused: u64,
}

/// `ticks` ticks of the bots over `strips`' workers.
fn run(start: &Strips, ticks: usize, trouble: Trouble) -> Run {
    let mut strips = start.clone();
    let mut layer = Layer::new(&strips, bots(200));
    let mut workers: Vec<Option<Worker<Bot>>> = (0..strips.workers() as WorkerId)
        .map(|w| Some(Worker::new(w)))
        .collect();
    // Revokes not yet heard: (tick until which the old owner keeps writing, worker, id, epoch).
    let mut late: Vec<(usize, WorkerId, EntityId, u32)> = Vec::new();
    let mut random = SplitMix64::new(11);
    let mut out = Run {
        states: Vec::with_capacity(ticks),
        handoffs: 0,
        reassigned: 0,
        owned: BTreeMap::new(),
        refused: 0,
    };
    for tick in 0..ticks {
        if trouble == Trouble::WorkerAway && tick == 400 {
            workers[1] = None;
            out.reassigned += layer.reassign(&strips, 1, 0).len();
        }
        if trouble == Trouble::WorkerAway && tick == 600 {
            workers[1] = Some(Worker::new(1));
            out.reassigned += layer.reassign(&strips, 1, 1).len();
        }
        // A worker leaving or joining: the strips cut again over the members, the handoffs at once.
        let members = match (trouble, tick) {
            (Trouble::Membership, 400) => Some(vec![0]),
            (Trouble::Membership, 600) => Some(vec![0, 2]),
            _ => None,
        };
        if let Some(members) = members {
            let xs = layer.records().values().map(|r| r.state.position.x);
            strips = Strips::balanced(members, xs, strips.margin, strips.band);
            workers.resize_with(3, || None);
            workers[1] = None;
            if tick == 600 {
                workers[2] = Some(Worker::new(2));
            }
            out.reassigned += layer.rebalance(&strips).len();
        }
        // The barrier: each worker takes its picture of the layer.
        for w in workers.iter_mut().flatten() {
            w.sync(&layer, &strips);
        }
        late.retain(|&(until, ..)| tick < until);
        for &(_, worker, id, epoch) in &late {
            if let Some(w) = workers[usize::from(worker)].as_mut()
                && let Some(r) = layer.records().get(&id)
            {
                w.keep(id, epoch, r.state.clone());
            }
        }
        // Each steps its own; the layer takes the writes in worker order.
        let writes: Vec<(WorkerId, Writes)> = workers
            .iter()
            .flatten()
            .map(|w| (w.id(), w.step(step)))
            .collect();
        for (worker, list) in writes {
            for (id, epoch, state) in list {
                let _ = layer.write(worker, id, epoch, state);
            }
        }
        let handoffs = layer.rebalance(&strips);
        if trouble == Trouble::LateRevokes {
            for h in &handoffs {
                let lag = random.below(3) as usize;
                if lag > 0 {
                    late.push((tick + 1 + lag, h.from, h.id, h.epoch - 1));
                }
            }
        }
        out.handoffs += handoffs.len();
        out.states.push(
            layer
                .records()
                .iter()
                .map(|(&id, r)| (id, r.state.clone()))
                .collect(),
        );
    }
    out.refused = layer.refused();
    for r in layer.records().values() {
        *out.owned.entry(r.owner).or_default() += 1;
    }
    out
}

/// Two workers either side of x = 0: an entity changes hands 2 m past the border, and a worker
/// sees 2 m and a neighbour's reach beyond its region.
fn two() -> Strips {
    Strips::new(vec![0.0], 2.0, 2.0 + REACH + 0.01)
}

/// One worker holding everything: the reference.
fn one() -> Strips {
    Strips::new(Vec::new(), 2.0, 0.0)
}

/// Where the split run first differs from the reference: its tick and entity.
fn first_difference(a: &Run, b: &Run) -> Option<(usize, EntityId)> {
    a.states
        .iter()
        .zip(&b.states)
        .enumerate()
        .find_map(|(tick, (x, y))| {
            x.iter()
                .zip(y)
                .find(|(p, q)| p != q)
                .map(|(p, _)| (tick, p.0))
        })
}

#[test]
fn two_workers_step_the_bots_as_one_does() {
    let reference = run(&one(), 1800, Trouble::None);
    let split = run(&two(), 1800, Trouble::None);
    eprintln!("two workers: {} handoffs in 1800 ticks", split.handoffs);
    assert_eq!(reference.handoffs, 0);
    assert!(split.handoffs > 200, "{} handoffs", split.handoffs);
    assert_eq!(split.refused, 0);
    assert_eq!(first_difference(&split, &reference), None);
}

#[test]
fn late_revokes_write_in_vain() {
    let reference = run(&one(), 1800, Trouble::None);
    let split = run(&two(), 1800, Trouble::LateRevokes);
    eprintln!(
        "late revokes: {} handoffs, {} writes refused",
        split.handoffs, split.refused
    );
    assert!(split.refused > 100, "{} writes refused", split.refused);
    assert_eq!(first_difference(&split, &reference), None);
}

#[test]
fn a_worker_away_is_covered_and_comes_back() {
    let reference = run(&one(), 1800, Trouble::None);
    let split = run(&two(), 1800, Trouble::WorkerAway);
    eprintln!(
        "a worker away: {} reassigned, {} handoffs besides",
        split.reassigned, split.handoffs
    );
    // Its region's entities went to worker 0 and back.
    assert!(split.reassigned > 150, "{} reassigned", split.reassigned);
    assert_eq!(first_difference(&split, &reference), None);
}

#[test]
fn a_worker_leaving_and_another_joining_cut_the_strips_again() {
    let reference = run(&one(), 1800, Trouble::None);
    let split = run(&two(), 1800, Trouble::Membership);
    eprintln!(
        "workers leaving and joining: {} handed over at the cuts, {} handoffs besides, owned at the end {:?}",
        split.reassigned, split.handoffs, split.owned
    );
    // Worker 1's bots went to worker 0 at once, and half of them to worker 2 when it came.
    assert!(split.reassigned > 150, "{} reassigned", split.reassigned);
    assert_eq!(split.owned.get(&1), None, "the worker gone owns nothing");
    let (a, b) = (split.owned[&0], split.owned[&2]);
    assert!(a.abs_diff(b) < 60, "{a} and {b}");
    assert_eq!(first_difference(&split, &reference), None);
}

//! Replication without sockets: a server's replicator and a client's receiver joined by two
//! seeded conditioned links on a virtual clock (issue #268, step 2).

use std::collections::BTreeMap;
use std::time::Duration;

use glam::DVec3;

use forge_net::crowd::Crowd;
use forge_net::{
    Ack, ConditionerConfig, EntityId, LinkConditioner, ReplicationConfig, Replicator,
    SnapshotReceiver, SnapshotStats,
};

const CLIENT: u64 = 1;
const TICK: f64 = 1.0 / 60.0;

/// One direction of a link on a virtual clock.
struct Link {
    conditioner: LinkConditioner,
    in_flight: Vec<(Duration, u64, Vec<u8>)>,
    order: u64,
}

impl Link {
    fn new(config: ConditionerConfig) -> Self {
        Self {
            conditioner: LinkConditioner::new(config),
            in_flight: Vec::new(),
            order: 0,
        }
    }

    fn send(&mut self, now: Duration, packet: Vec<u8>) {
        for at in self
            .conditioner
            .submit(now, packet.len())
            .into_iter()
            .flatten()
        {
            self.in_flight.push((at, self.order, packet.clone()));
            self.order += 1;
        }
    }

    fn deliver(&mut self, now: Duration) -> Vec<Vec<u8>> {
        let mut due: Vec<_> = Vec::new();
        let mut k = 0;
        while k < self.in_flight.len() {
            if self.in_flight[k].0 <= now {
                due.push(self.in_flight.swap_remove(k));
            } else {
                k += 1;
            }
        }
        due.sort_by_key(|d| (d.0, d.1));
        due.into_iter().map(|d| d.2).collect()
    }
}

fn bad_link(seed: u64) -> ConditionerConfig {
    ConditionerConfig::new(50, 0.05, seed)
        .with_jitter(Duration::from_millis(10))
        .with_duplicate(0.02)
        .with_reorder(0.05, Duration::from_millis(20))
}

/// A server, one client at the origin, and the links between them.
struct Run {
    crowd: Crowd,
    replicator: Replicator,
    receiver: SnapshotReceiver,
    down: Link,
    up: Link,
    tick: u64,
    /// Every snapshot's bytes and what it held.
    sent: Vec<(Vec<u8>, SnapshotStats)>,
    /// How many times each entity arrived.
    arrivals: BTreeMap<EntityId, u32>,
}

impl Run {
    fn new(crowd: Crowd, config: ReplicationConfig, seed: u64) -> Self {
        let mut replicator = Replicator::new(config);
        replicator.add_client(CLIENT, DVec3::ZERO);
        Self {
            crowd,
            replicator,
            receiver: SnapshotReceiver::new(config.quantization),
            down: Link::new(bad_link(seed)),
            up: Link::new(bad_link(seed + 1)),
            tick: 0,
            sent: Vec::new(),
            arrivals: BTreeMap::new(),
        }
    }

    /// Runs `seconds` at 60 Hz with a snapshot every `every` ticks; the crowd moves if `moving`.
    fn run(&mut self, seconds: f64, every: u64, moving: bool) {
        let ticks = (seconds / TICK).round() as u64;
        for _ in 0..ticks {
            let now = Duration::from_secs_f64(self.tick as f64 * TICK);
            if moving {
                self.crowd.step(TICK);
            }
            if self.tick.is_multiple_of(every) {
                self.replicator
                    .set_world(self.crowd.states().iter().map(|(id, s)| (*id, s, 1.0)));
                let (bytes, stats) = self
                    .replicator
                    .snapshot(CLIENT, self.tick as u32)
                    .expect("a known client");
                assert!(
                    bytes.len() <= self.replicator.config().packet_budget,
                    "{} bytes over the budget",
                    bytes.len()
                );
                if stats.deltas > 0 {
                    assert_eq!(stats.waiting, 0, "a delta went out before an initial state");
                }
                self.down.send(now, bytes.clone());
                self.sent.push((bytes, stats));
            }
            for p in self.down.deliver(now) {
                if let forge_net::Taken::Applied { .. } = self.receiver.receive(&p).unwrap() {
                    for id in self.receiver.latest_entities() {
                        *self.arrivals.entry(id).or_default() += 1;
                    }
                }
            }
            if let Some(ack) = self.receiver.ack() {
                self.up.send(now, ack.encode().to_vec());
            }
            for a in self.up.deliver(now) {
                self.replicator
                    .acknowledge(CLIENT, Ack::decode(&a).expect("six bytes"));
            }
            self.tick += 1;
        }
    }

    /// Checks that the client holds exactly the server's state of everyone within the radius,
    /// and nobody beyond the radius and its margin.
    fn check_converged(&self, at_least: usize) {
        let config = *self.replicator.config();
        let q = config.quantization;
        let view = self.receiver.view();
        let mut inside = 0;
        for (id, s) in self.crowd.states() {
            let d = s.position.length();
            if d <= config.radius {
                inside += 1;
                assert_eq!(
                    view.get(id),
                    Some(&q.quantize(s)),
                    "entity {id} at {d:.1} m"
                );
            } else if d > config.radius + config.margin {
                assert!(
                    !view.contains_key(id),
                    "entity {id} at {d:.1} m is still held"
                );
            }
        }
        assert!(inside >= at_least, "only {inside} entities in interest");
    }
}

#[test]
fn a_client_converges_to_the_servers_state_over_a_bad_link() {
    let mut run = Run::new(Crowd::new(3000, 600.0, 1), ReplicationConfig::default(), 10);
    run.run(20.0, 2, true);
    let moving: Vec<SnapshotStats> = run.sent.iter().map(|s| s.1).collect();
    let joined = moving
        .iter()
        .position(|s| s.deltas > 0)
        .expect("deltas flowed");
    assert!(
        moving[..joined].iter().all(|s| s.deltas == 0 && s.full > 0),
        "the join sends full states only"
    );
    run.crowd.freeze();
    run.run(4.0, 2, false);
    run.check_converged(300);
    let removals: usize = moving.iter().map(|s| s.removals).sum();
    assert!(removals > 0, "walkers left the interest");
}

#[test]
fn the_same_world_and_link_give_the_same_bytes() {
    let go = || {
        let mut run = Run::new(Crowd::new(1500, 500.0, 2), ReplicationConfig::default(), 20);
        run.run(5.0, 2, true);
        run.sent.into_iter().map(|s| s.0).collect::<Vec<_>>()
    };
    let (a, b) = (go(), go());
    assert_eq!(a.len(), 150);
    assert!(a == b, "two runs sent different bytes");
}

#[test]
fn near_entities_are_sent_more_often_than_far_ones() {
    let config = ReplicationConfig {
        packet_budget: 300,
        ..ReplicationConfig::default()
    };
    let mut run = Run::new(Crowd::new(3000, 600.0, 3), config, 30);
    run.run(10.0, 2, true);
    // Arrivals by where each entity ended: a walker moves 15 m at most in 10 s.
    let (mut near, mut far) = ((0u32, 0u32), (0u32, 0u32));
    for (id, s) in run.crowd.states() {
        let d = s.position.length();
        let n = run.arrivals.get(id).copied().unwrap_or(0);
        if d < 30.0 {
            near = (near.0 + n, near.1 + 1);
        } else if (100.0..135.0).contains(&d) {
            far = (far.0 + n, far.1 + 1);
        }
    }
    let rate = |(sum, count): (u32, u32)| f64::from(sum) / f64::from(count.max(1)) / 10.0;
    let (near, far) = (rate(near), rate(far));
    println!("updates a second: {near:.1} within 30 m, {far:.1} at 100-135 m");
    assert!(near > 2.0 * far, "near {near}, far {far}");
}

#[test]
fn entities_that_leave_are_forgotten_and_those_that_return_come_back_whole() {
    let mut crowd = Crowd::new(200, 200.0, 4);
    crowd.freeze();
    let mut run = Run::new(crowd, ReplicationConfig::default(), 40);
    run.run(2.0, 2, false);
    run.check_converged(200);
    // Half go far away; their slots are freed and given to others coming in.
    for k in 0..100 {
        run.crowd.place(k, DVec3::new(1000.0 + k as f64, 0.0, 0.0));
    }
    run.run(2.0, 2, false);
    run.check_converged(100);
    for k in 0..100 {
        run.crowd.place(k, DVec3::new(-50.0 + k as f64, 0.0, 30.0));
    }
    run.run(2.0, 2, false);
    run.check_converged(200);
    assert_eq!(run.receiver.view().len(), 200);
}

#[test]
fn a_lost_removal_and_a_reused_slot_do_not_lose_an_entity() {
    let config = ReplicationConfig::default();
    let mut crowd = Crowd::new(3, 100.0, 9);
    crowd.freeze();
    crowd.place(0, DVec3::new(10.0, 0.0, 0.0));
    crowd.place(1, DVec3::new(20.0, 0.0, 0.0));
    crowd.place(2, DVec3::new(1000.0, 0.0, 0.0));
    let mut replicator = Replicator::new(config);
    replicator.add_client(CLIENT, DVec3::ZERO);
    let mut receiver = SnapshotReceiver::new(config.quantization);
    let mut round = |crowd: &Crowd, deliver: bool, tick: u32| {
        replicator.set_world(crowd.states().iter().map(|(id, s)| (*id, s, 1.0)));
        let (bytes, _) = replicator.snapshot(CLIENT, tick).unwrap();
        if deliver {
            receiver.receive(&bytes).unwrap();
            replicator.acknowledge(CLIENT, receiver.ack().unwrap());
        }
        receiver.view().clone()
    };
    for t in 0..5 {
        round(&crowd, true, t);
    }
    // Person 0 leaves as person 2 arrives and takes its slot; that packet is lost.
    crowd.place(0, DVec3::new(-1000.0, 0.0, 0.0));
    crowd.place(2, DVec3::new(30.0, 0.0, 0.0));
    round(&crowd, false, 5);
    // Person 0 comes back under a new slot before its removal went out again.
    crowd.place(0, DVec3::new(10.0, 0.0, 5.0));
    let mut view = BTreeMap::new();
    for t in 6..30 {
        view = round(&crowd, true, t);
    }
    let q = config.quantization;
    for (id, s) in crowd.states() {
        assert_eq!(view.get(id), Some(&q.quantize(s)), "entity {id}");
    }
}

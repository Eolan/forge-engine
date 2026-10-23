//! Lag compensation end to end (issue #268, step 3): the crowd replicated over a bad link to a
//! client that draws it a fixed delay behind the server's clock (`forge_net::interpolate`), shots
//! aimed where the client drew a person, and the server testing each against its hit history at
//! the moment the client drew (`forge_sim::rewind`), on a virtual clock.

use std::time::Duration;

use glam::DVec3;

use forge_net::crowd::Crowd;
use forge_net::{
    Ack, ConditionerConfig, EntityId, Interpolator, LinkConditioner, Moment, PlayoutClock,
    ReplicationConfig, Replicator, SnapshotReceiver,
};
use forge_sim::rewind::{Capsule, History, View};

const TICK: f64 = 1.0 / 60.0;
/// A snapshot every other tick: 30 Hz.
const EVERY: u64 = 2;
/// Ticks the client draws behind the server's clock: three snapshot intervals, and one for the
/// link's jitter.
const DELAY: f64 = 7.0;
/// A person's hit shape.
const HEIGHT: f64 = 1.8;
const RADIUS: f64 = 0.3;
const REACH: f64 = 100.0;

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

/// A link of `ms` one way, a fifth of it as jitter, 5 % lost, 2 % duplicated, 5 % held back 20 ms.
fn link(ms: u64, seed: u64) -> ConditionerConfig {
    ConditionerConfig::new(ms, 0.05, seed)
        .with_jitter(Duration::from_millis(ms / 5))
        .with_duplicate(0.02)
        .with_reorder(0.05, Duration::from_millis(20))
}

/// A shot as the client sends it: the moment it drew, its direction, and what it saw along it
/// (the person and the distance), for the test to compare.
struct Shot {
    moment: Moment,
    direction: DVec3,
    seen: EntityId,
    distance: f64,
}

impl Shot {
    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(52);
        b.extend_from_slice(&self.moment.tick.to_le_bytes());
        b.extend_from_slice(&self.moment.fraction.to_le_bytes());
        for v in self.direction.to_array() {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&self.seen.to_le_bytes());
        b.extend_from_slice(&self.distance.to_le_bytes());
        b
    }

    fn decode(b: &[u8]) -> Self {
        let f = |k: usize| f64::from_le_bytes(b[k..k + 8].try_into().expect("eight bytes"));
        Self {
            moment: Moment {
                tick: u64::from_le_bytes(b[0..8].try_into().expect("eight bytes")),
                fraction: f(8),
            },
            direction: DVec3::new(f(16), f(24), f(32)),
            seen: u32::from_le_bytes(b[40..44].try_into().expect("four bytes")),
            distance: f(44),
        }
    }
}

/// The nearest person along a ray from `eye` among `people` (feet positions by id), and how far.
fn nearest(eye: DVec3, people: &[(EntityId, DVec3)], direction: DVec3) -> Option<(EntityId, f64)> {
    people
        .iter()
        .filter_map(|&(id, feet)| {
            Capsule::standing(feet, HEIGHT, RADIUS)
                .ray(eye, direction, REACH)
                .map(|d| (id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
}

/// What the server made of a client's shots: how many arrived, and of them how many met the person
/// the client saw, at the client's moment and at the server's present.
#[derive(Debug, Default)]
struct Tally {
    arrived: u32,
    rewound_same: u32,
    present_same: u32,
    /// The rewound hit's distance against the client's, each shot (metres).
    errors: Vec<f64>,
}

impl Tally {
    /// The error at quantile `q`, metres.
    fn error(&self, q: f64) -> f64 {
        let mut errors = self.errors.clone();
        errors.sort_by(f64::total_cmp);
        errors[((errors.len() - 1) as f64 * q) as usize]
    }
}

/// A client: its links, what it holds of the crowd, its clock, where it stands and its tally.
struct Player {
    id: u64,
    eye: DVec3,
    down: Link,
    up: Link,
    shots: Link,
    receiver: SnapshotReceiver,
    interpolator: Interpolator,
    clock: PlayoutClock,
    tally: Tally,
}

/// 400 people in a 120 m square, 20 s; the clients `players` (each its link's one-way delay, ms,
/// and where it stands on the ground), each over its own bad link both ways. From 2 s in, every 6
/// ticks each client shoots at the chest of the nearest person it draws 5–30 m away.
fn play(players: &[(u64, DVec3)]) -> Vec<Tally> {
    let config = ReplicationConfig::default();
    let q = config.quantization;
    let mut crowd = Crowd::new(400, 120.0, 3);
    let mut replicator = Replicator::new(config);
    let mut history = History::new(60);
    let mut players: Vec<Player> = players
        .iter()
        .enumerate()
        .map(|(k, &(ms, at))| {
            let id = k as u64 + 1;
            let seed = 30 + 10 * k as u64;
            replicator.add_client(id, at);
            Player {
                id,
                eye: at + DVec3::Y * 1.6,
                down: Link::new(link(ms, seed)),
                up: Link::new(link(ms, seed + 1)),
                shots: Link::new(link(ms, seed + 2)),
                receiver: SnapshotReceiver::new(q),
                interpolator: Interpolator::new(q, 60),
                clock: PlayoutClock::new(DELAY, 30),
                tally: Tally::default(),
            }
        })
        .collect();
    for tick in 0..(20.0 / TICK) as u64 {
        let now = Duration::from_secs_f64(tick as f64 * TICK);
        crowd.step(TICK);
        history.record(
            tick,
            crowd
                .states()
                .iter()
                .map(|(id, s)| (*id, Capsule::standing(s.position, HEIGHT, RADIUS))),
        );
        if tick.is_multiple_of(EVERY) {
            replicator.set_world(crowd.states().iter().map(|(id, s)| (*id, s, 1.0)));
        }
        for p in &mut players {
            if tick.is_multiple_of(EVERY) {
                let (bytes, _) = replicator
                    .snapshot(p.id, tick as u32)
                    .expect("a known client");
                p.down.send(now, bytes);
            }
            for packet in p.down.deliver(now) {
                if let forge_net::Taken::Applied { .. } =
                    p.receiver.receive(&packet).expect("a snapshot")
                {
                    p.interpolator.take(&p.receiver);
                    p.clock.arrived(tick as f64, p.receiver.tick());
                }
            }
            if let Some(ack) = p.receiver.ack() {
                p.up.send(now, ack.encode().to_vec());
            }
            for a in p.up.deliver(now) {
                replicator.acknowledge(p.id, Ack::decode(&a).expect("six bytes"));
            }
            // The client shoots at what it draws.
            if tick >= 120
                && tick.is_multiple_of(6)
                && let Some(moment) = p.clock.moment(tick as f64)
            {
                let drawn = p.interpolator.at(moment);
                let eye = p.eye;
                let target = drawn
                    .iter()
                    .map(|&(id, feet)| (id, feet, (feet - eye).with_y(0.0).length()))
                    .filter(|t| (5.0..30.0).contains(&t.2))
                    .min_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
                if let Some((_, feet, _)) = target {
                    let direction = (feet + DVec3::Y * 1.2 - eye).normalize();
                    if let Some((seen, distance)) = nearest(eye, &drawn, direction) {
                        let shot = Shot {
                            moment,
                            direction,
                            seen,
                            distance,
                        };
                        p.shots.send(now, shot.encode());
                    }
                }
            }
            // The server tests each shot as it arrives.
            for s in p.shots.deliver(now) {
                let shot = Shot::decode(&s);
                p.tally.arrived += 1;
                let view = View {
                    tick: shot.moment.tick,
                    fraction: shot.moment.fraction,
                };
                let rewound = history.ray(view, p.eye, shot.direction, REACH, None);
                if let Some(hit) = rewound.filter(|h| h.id == shot.seen) {
                    p.tally.rewound_same += 1;
                    p.tally.errors.push((hit.distance - shot.distance).abs());
                }
                let present = View {
                    tick,
                    fraction: 0.0,
                };
                if history
                    .ray(present, p.eye, shot.direction, REACH, None)
                    .is_some_and(|h| h.id == shot.seen)
                {
                    p.tally.present_same += 1;
                }
            }
        }
    }
    players.into_iter().map(|p| p.tally).collect()
}

/// Rewound to the moment the client drew, the server meets the person the client aimed at, at
/// the distance the client saw, within the quantisation and the interpolation (millimetres).
/// Tested where the person stands when the shot arrives, a good share of the same shots miss.
#[test]
fn a_rewound_shot_meets_the_person_where_the_client_drew_it() {
    let tally = play(&[(50, DVec3::ZERO)]).remove(0);
    eprintln!(
        "{} shots arrived; rewound: {} met the person seen, error p50 {:.4} p99 {:.4} max {:.4} m; \
         at the present: {}",
        tally.arrived,
        tally.rewound_same,
        tally.error(0.5),
        tally.error(0.99),
        tally.error(1.0),
        tally.present_same
    );
    assert!(tally.arrived > 150, "{} shots arrived", tally.arrived);
    assert!(
        f64::from(tally.rewound_same) >= 0.99 * f64::from(tally.arrived),
        "{} of {} rewound shots met the person seen",
        tally.rewound_same,
        tally.arrived
    );
    assert!(
        tally.error(0.99) < 0.01,
        "the rewound distance off by {:.4} m",
        tally.error(0.99)
    );
    assert!(
        f64::from(tally.present_same) < 0.9 * f64::from(tally.arrived),
        "{} of {} shots met the person at the present too: the test shows nothing",
        tally.present_same,
        tally.arrived
    );
}

/// D-010's hit-registration stage: eight clients over links from 10 to 125 ms one way (round
/// trips of 20 to 250 ms), spread over the square, each shooting at the people it draws: the
/// rewound shots meet the person each saw, whatever its link.
#[test]
fn eight_clients_from_20_to_250_ms_hit_what_they_saw() {
    let players: Vec<(u64, DVec3)> = [10, 25, 40, 55, 70, 90, 110, 125]
        .iter()
        .enumerate()
        .map(|(k, &ms)| {
            let angle = k as f64 * std::f64::consts::TAU / 8.0;
            (ms, DVec3::new(angle.cos(), 0.0, angle.sin()) * 30.0)
        })
        .collect();
    let tallies = play(&players);
    for ((ms, _), t) in players.iter().zip(&tallies) {
        eprintln!(
            "{} ms each way: {} shots, {} met the person seen (p99 {:.4} m), {} at the present",
            ms,
            t.arrived,
            t.rewound_same,
            t.error(0.99),
            t.present_same
        );
        assert!(t.arrived > 150, "{ms} ms: {} shots arrived", t.arrived);
        assert!(
            f64::from(t.rewound_same) >= 0.98 * f64::from(t.arrived),
            "{ms} ms: {} of {} met the person seen",
            t.rewound_same,
            t.arrived
        );
        assert!(
            t.error(0.99) < 0.01,
            "{ms} ms: off by {:.4} m",
            t.error(0.99)
        );
    }
}

/// The same crowd, links and shots give the same tallies.
#[test]
fn the_same_run_gives_the_same_shots() {
    let (a, b) = (
        play(&[(50, DVec3::ZERO)]).remove(0),
        play(&[(50, DVec3::ZERO)]).remove(0),
    );
    assert_eq!(
        (a.arrived, a.rewound_same, a.present_same),
        (b.arrived, b.rewound_same, b.present_same)
    );
    assert_eq!(a.errors, b.errors);
}

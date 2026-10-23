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

const CLIENT: u64 = 1;
const TICK: f64 = 1.0 / 60.0;
/// A snapshot every other tick: 30 Hz.
const EVERY: u64 = 2;
/// Ticks the client draws behind the server's clock: three snapshot intervals, and one for the
/// link's jitter.
const DELAY: f64 = 7.0;
/// A person's hit shape.
const HEIGHT: f64 = 1.8;
const RADIUS: f64 = 0.3;
/// The shooter's eye, at the client's focus.
const EYE: DVec3 = DVec3::new(0.0, 1.6, 0.0);
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

/// 50 ms, 10 ms of jitter, 5 % lost, 2 % duplicated, 5 % held back 20 ms.
fn bad_link(seed: u64) -> ConditionerConfig {
    ConditionerConfig::new(50, 0.05, seed)
        .with_jitter(Duration::from_millis(10))
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

/// The nearest person along a ray among `people` (feet positions by id), and how far.
fn nearest(people: &[(EntityId, DVec3)], direction: DVec3) -> Option<(EntityId, f64)> {
    people
        .iter()
        .filter_map(|&(id, feet)| {
            Capsule::standing(feet, HEIGHT, RADIUS)
                .ray(EYE, direction, REACH)
                .map(|d| (id, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
}

/// What the server made of the shots: how many arrived, and of them how many met the person the
/// client saw, at the client's moment and at the server's present.
#[derive(Debug, Default)]
struct Tally {
    arrived: u32,
    rewound_same: u32,
    present_same: u32,
    /// The rewound hit's distance against the client's, each shot (metres).
    errors: Vec<f64>,
}

/// 400 people in a 120 m square, the client at its middle, 20 s over the bad link both ways; from
/// 2 s in, every 6 ticks the client shoots at the chest of the nearest person it draws between 5
/// and 30 m away.
fn play() -> Tally {
    let config = ReplicationConfig::default();
    let q = config.quantization;
    let mut crowd = Crowd::new(400, 120.0, 3);
    let mut replicator = Replicator::new(config);
    replicator.add_client(CLIENT, DVec3::ZERO);
    let mut receiver = SnapshotReceiver::new(q);
    let mut interpolator = Interpolator::new(q, 60);
    let mut clock = PlayoutClock::new(DELAY, 30);
    let mut history = History::new(60);
    let mut down = Link::new(bad_link(30));
    let mut up = Link::new(bad_link(31));
    let mut shots = Link::new(bad_link(32));
    let mut tally = Tally::default();
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
            let (bytes, _) = replicator
                .snapshot(CLIENT, tick as u32)
                .expect("a known client");
            down.send(now, bytes);
        }
        for p in down.deliver(now) {
            if let forge_net::Taken::Applied { .. } = receiver.receive(&p).expect("a snapshot") {
                interpolator.take(&receiver);
                clock.arrived(tick as f64, receiver.tick());
            }
        }
        if let Some(ack) = receiver.ack() {
            up.send(now, ack.encode().to_vec());
        }
        for a in up.deliver(now) {
            replicator.acknowledge(CLIENT, Ack::decode(&a).expect("six bytes"));
        }
        // The client shoots at what it draws.
        if tick >= 120
            && tick.is_multiple_of(6)
            && let Some(moment) = clock.moment(tick as f64)
        {
            let drawn = interpolator.at(moment);
            let target = drawn
                .iter()
                .map(|&(id, feet)| (id, feet, (feet - EYE).with_y(0.0).length()))
                .filter(|p| (5.0..30.0).contains(&p.2))
                .min_by(|a, b| a.2.total_cmp(&b.2).then(a.0.cmp(&b.0)));
            if let Some((_, feet, _)) = target {
                let direction = (feet + DVec3::Y * 1.2 - EYE).normalize();
                if let Some((seen, distance)) = nearest(&drawn, direction) {
                    let shot = Shot {
                        moment,
                        direction,
                        seen,
                        distance,
                    };
                    shots.send(now, shot.encode());
                }
            }
        }
        // The server tests each shot as it arrives.
        for s in shots.deliver(now) {
            let shot = Shot::decode(&s);
            tally.arrived += 1;
            let view = View {
                tick: shot.moment.tick,
                fraction: shot.moment.fraction,
            };
            let rewound = history.ray(view, EYE, shot.direction, REACH, None);
            if let Some(hit) = rewound.filter(|h| h.id == shot.seen) {
                tally.rewound_same += 1;
                tally.errors.push((hit.distance - shot.distance).abs());
            }
            let present = View {
                tick,
                fraction: 0.0,
            };
            if history
                .ray(present, EYE, shot.direction, REACH, None)
                .is_some_and(|h| h.id == shot.seen)
            {
                tally.present_same += 1;
            }
        }
    }
    tally
}

/// Rewound to the moment the client drew, the server meets the person the client aimed at, at
/// the distance the client saw, within the quantisation and the interpolation (millimetres).
/// Tested where the person stands when the shot arrives, a good share of the same shots miss.
#[test]
fn a_rewound_shot_meets_the_person_where_the_client_drew_it() {
    let tally = play();
    let mut errors = tally.errors.clone();
    errors.sort_by(f64::total_cmp);
    let p = |q: f64| errors[((errors.len() - 1) as f64 * q) as usize];
    eprintln!(
        "{} shots arrived; rewound: {} met the person seen, error p50 {:.4} p99 {:.4} max {:.4} m; \
         at the present: {}",
        tally.arrived,
        tally.rewound_same,
        p(0.5),
        p(0.99),
        errors.last().copied().unwrap_or(0.0),
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
        p(0.99) < 0.01,
        "the rewound distance off by {:.4} m",
        p(0.99)
    );
    assert!(
        f64::from(tally.present_same) < 0.9 * f64::from(tally.arrived),
        "{} of {} shots met the person at the present too: the test shows nothing",
        tally.present_same,
        tally.arrived
    );
}

/// The same crowd, link and shots give the same tally.
#[test]
fn the_same_run_gives_the_same_shots() {
    let (a, b) = (play(), play());
    assert_eq!(
        (a.arrived, a.rewound_same, a.present_same),
        (b.arrived, b.rewound_same, b.present_same)
    );
    assert_eq!(a.errors, b.errors);
}

//! People and vehicles going by: what the animals react to. A game gives the fauna its own
//! people (the population's walkers, the player) and traffic each step; [`Passers`] makes an
//! example day of them from a plan's streets, each a pure function of the time of day.

use forge_core::SplitMix64;
use glam::{Vec2, Vec3};

use crate::plan::Plan;

/// What disturbs an animal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Threat {
    /// A person on foot.
    Person,
    /// A vehicle on a street limited to `limit` km/h.
    Vehicle {
        /// The street's limit, km/h.
        limit: f32,
    },
    /// A dog.
    Dog,
    /// A cat.
    Cat,
}

/// Someone or something an animal may flee.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disturber {
    /// Where it is (`z` its middle's height).
    pub pos: Vec3,
    /// Its velocity, m/s.
    pub vel: Vec3,
    /// What it is.
    pub threat: Threat,
}

/// One person's or vehicle's trip along the streets.
#[derive(Clone, Debug, PartialEq)]
pub struct Passer {
    /// What it is.
    pub threat: Threat,
    /// Its points, the street nodes it passes.
    pub points: Vec<Vec2>,
    /// When it reaches each point, seconds since midnight.
    pub times: Vec<f32>,
}

impl Passer {
    /// When it sets out.
    pub fn start(&self) -> f32 {
        self.times[0]
    }

    /// When it arrives.
    pub fn end(&self) -> f32 {
        self.times[self.times.len() - 1]
    }

    /// Where it is at `t` and how fast it goes, if under way.
    pub fn at(&self, t: f32) -> Option<(Vec2, Vec2)> {
        if t < self.start() || t >= self.end() {
            return None;
        }
        let k = self
            .times
            .partition_point(|&x| x <= t)
            .clamp(1, self.times.len() - 1);
        let (t0, t1) = (self.times[k - 1], self.times[k]);
        let (a, b) = (self.points[k - 1], self.points[k]);
        let dt = (t1 - t0).max(1e-3);
        let s = (t - t0) / dt;
        Some((a + (b - a) * s, (b - a) / dt))
    }
}

/// Starts per hour, as shares of the busiest: few at night, the morning, noon and the evening
/// stroll (Forge's estimate for a Mediterranean village in June).
const PEOPLE_BY_HOUR: [f32; 24] = [
    0.15, 0.08, 0.04, 0.03, 0.05, 0.12, 0.35, 0.6, 0.8, 0.75, 0.7, 0.8, 0.85, 0.6, 0.45, 0.5, 0.6,
    0.75, 0.9, 1.0, 1.0, 0.85, 0.55, 0.3,
];

/// A day's passers, ordered by their start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Passers {
    /// The trips.
    pub list: Vec<Passer>,
    longest: f32,
}

impl Passers {
    /// An example day on `plan`'s streets from `seed`: `people` walks at the busiest hour, each
    /// a random walk of 3–10 streets at 1.1–1.5 m/s; `vehicles` at the busiest hour along the
    /// streets with a limit of 30 km/h or more, at 90 % of each street's limit.
    pub fn example(plan: &Plan, people: u32, vehicles: u32, seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed ^ 0x9A55_E125);
        let mut list = Vec::new();
        let nodes = &plan.parts.nodes;
        let streets = &plan.parts.streets;
        for (hour, &share) in PEOPLE_BY_HOUR.iter().enumerate() {
            for _ in 0..(people as f32 * share).round() as u32 {
                let start = (hour as f32 + rng.next_f32()) * 3600.0;
                let speed = rng.range_f32(1.1, 1.5);
                let mut n = rng.below(nodes.len() as u64) as u32;
                let mut points = vec![nodes[n as usize]];
                let mut times = vec![start];
                let mut came = u32::MAX;
                for _ in 0..rng.range_f32(3.0, 10.0) as u32 {
                    let out: Vec<u32> = plan.adjacent[n as usize]
                        .iter()
                        .copied()
                        .filter(|&e| e != came)
                        .collect();
                    if out.is_empty() {
                        break;
                    }
                    let e = out[rng.below(out.len() as u64) as usize];
                    let m = plan.other(e, n);
                    let d = (nodes[m as usize] - nodes[n as usize]).length();
                    points.push(nodes[m as usize]);
                    times.push(times[times.len() - 1] + d / speed);
                    came = e;
                    n = m;
                }
                if points.len() > 1 {
                    list.push(Passer {
                        threat: Threat::Person,
                        points,
                        times,
                    });
                }
            }
            for _ in 0..(vehicles as f32 * share).round() as u32 {
                let start = (hour as f32 + rng.next_f32()) * 3600.0;
                // From a dead end of the road network through to another.
                let roads: Vec<u32> = (0..streets.len() as u32)
                    .filter(|&e| streets[e as usize].limit >= 30.0)
                    .collect();
                if roads.is_empty() {
                    break;
                }
                let ends: Vec<u32> = (0..nodes.len() as u32)
                    .filter(|&n| {
                        plan.adjacent[n as usize]
                            .iter()
                            .filter(|&&e| streets[e as usize].limit >= 30.0)
                            .count()
                            == 1
                    })
                    .collect();
                if ends.is_empty() {
                    break;
                }
                let mut n = ends[rng.below(ends.len() as u64) as usize];
                let mut points = vec![nodes[n as usize]];
                let mut times = vec![start];
                let mut came = u32::MAX;
                let mut limit = 30.0f32;
                loop {
                    let next = plan.adjacent[n as usize]
                        .iter()
                        .copied()
                        .find(|&e| e != came && streets[e as usize].limit >= 30.0);
                    let Some(e) = next else { break };
                    let m = plan.other(e, n);
                    limit = streets[e as usize].limit;
                    let d = (nodes[m as usize] - nodes[n as usize]).length();
                    points.push(nodes[m as usize]);
                    times.push(times[times.len() - 1] + d / (limit / 3.6 * 0.9));
                    came = e;
                    n = m;
                }
                if points.len() > 1 {
                    list.push(Passer {
                        threat: Threat::Vehicle { limit },
                        points,
                        times,
                    });
                }
            }
        }
        list.sort_by(|a, b| a.start().total_cmp(&b.start()));
        let longest = list.iter().map(|p| p.end() - p.start()).fold(0.0, f32::max);
        Self { list, longest }
    }

    /// Everyone under way at `t` (seconds since midnight, any day: the same day repeats),
    /// appended to `out`. A vehicle's limit is its current street's.
    pub fn at(&self, t: f32, plan: &Plan, out: &mut Vec<Disturber>) {
        let t = t % 86_400.0;
        let hi = self.list.partition_point(|p| p.start() <= t);
        let lo = self.list.partition_point(|p| p.start() < t - self.longest);
        for p in &self.list[lo..hi] {
            if let Some((pos, vel)) = p.at(t) {
                let (threat, z) = match p.threat {
                    Threat::Vehicle { .. } => (
                        Threat::Vehicle {
                            limit: street_limit(plan, pos),
                        },
                        0.8,
                    ),
                    other => (other, 1.0),
                };
                out.push(Disturber {
                    pos: pos.extend(z),
                    vel: vel.extend(0.0),
                    threat,
                });
            }
        }
    }
}

/// The limit of the street nearest `p`, km/h.
fn street_limit(plan: &Plan, p: Vec2) -> f32 {
    let nodes = &plan.parts.nodes;
    let mut best = (f32::MAX, 30.0);
    for s in &plan.parts.streets {
        if s.limit <= 0.0 {
            continue;
        }
        let (a, b) = (nodes[s.a as usize], nodes[s.b as usize]);
        let d = (crate::plan::nearest_on_segment(p, a, b) - p).length_squared();
        if d < best.0 {
            best = (d, s.limit);
        }
    }
    best.1
}

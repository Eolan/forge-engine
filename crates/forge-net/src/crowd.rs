//! A synthetic crowd for the replication's tests and measurements: people walking about a
//! square, standing, turning, from a seed.
//!
//! Half of them walk at 1.2 to 1.6 m/s, a quarter stroll at 0.5 to 0.9 m/s and a quarter stand;
//! a walker turns now and then (about every 8 s) and turns back at the square's edge. Ids are
//! 97 apart on average: the people near a client are a few among many, as in a city's
//! population (#250), so their ids are far apart. Not a simulation of anything: a stand-in with
//! the motion statistics that matter to the bytes (how many move, how fast, how often they
//! turn).

use glam::{DVec3, Quat, Vec3};

use forge_core::SplitMix64;

use crate::quantize::EntityState;
use crate::snapshot::EntityId;

/// One person.
#[derive(Clone, Copy, Debug)]
struct Walker {
    id: EntityId,
    position: DVec3,
    heading: f64,
    speed: f64,
}

/// The crowd (see the module's documentation).
#[derive(Clone, Debug)]
pub struct Crowd {
    walkers: Vec<Walker>,
    half: f64,
    random: SplitMix64,
    states: Vec<(EntityId, EntityState)>,
}

impl Crowd {
    /// `n` people in a square `side` metres wide centred on the origin.
    pub fn new(n: usize, side: f64, seed: u64) -> Self {
        let mut random = SplitMix64::new(seed);
        let half = side / 2.0;
        let walkers = (0..n)
            .map(|k| {
                let kind = random.below(4);
                Walker {
                    // 97 apart on average, unique.
                    id: (k as u32).wrapping_mul(97) + (random.below(97) as u32) + 1,
                    position: DVec3::new(
                        random.range_f64(-half, half),
                        0.0,
                        random.range_f64(-half, half),
                    ),
                    heading: random.range_f64(0.0, std::f64::consts::TAU),
                    speed: match kind {
                        0 | 1 => random.range_f64(1.2, 1.6),
                        2 => random.range_f64(0.5, 0.9),
                        _ => 0.0,
                    },
                }
            })
            .collect();
        let mut crowd = Self {
            walkers,
            half,
            random,
            states: Vec::with_capacity(n),
        };
        crowd.refresh();
        crowd
    }

    /// People in the crowd.
    pub fn len(&self) -> usize {
        self.walkers.len()
    }

    /// Whether the crowd is empty.
    pub fn is_empty(&self) -> bool {
        self.walkers.is_empty()
    }

    /// Moves everyone by `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        let turn_chance = dt / 8.0;
        for w in &mut self.walkers {
            if w.speed == 0.0 {
                continue;
            }
            if self.random.next_f64() < turn_chance {
                w.heading += self.random.range_f64(-1.5, 1.5);
            }
            let (s, c) = (libm::sin(w.heading), libm::cos(w.heading));
            w.position.x += s * w.speed * dt;
            w.position.z += c * w.speed * dt;
            if w.position.x.abs() > self.half || w.position.z.abs() > self.half {
                w.heading += std::f64::consts::PI;
                w.position.x = w.position.x.clamp(-self.half, self.half);
                w.position.z = w.position.z.clamp(-self.half, self.half);
            }
        }
        self.refresh();
    }

    /// Stops everyone where they are.
    pub fn freeze(&mut self) {
        for w in &mut self.walkers {
            w.speed = 0.0;
        }
        self.refresh();
    }

    /// Moves one person (by index) to `position`.
    pub fn place(&mut self, index: usize, position: DVec3) {
        self.walkers[index].position = position;
        self.refresh();
    }

    /// Each person's id and state.
    pub fn states(&self) -> &[(EntityId, EntityState)] {
        &self.states
    }

    fn refresh(&mut self) {
        self.states.clear();
        for w in &self.walkers {
            let (s, c) = (libm::sin(w.heading), libm::cos(w.heading));
            let velocity = Vec3::new((s * w.speed) as f32, 0.0, (c * w.speed) as f32);
            self.states.push((
                w.id,
                EntityState {
                    position: w.position,
                    rotation: Quat::from_xyzw(
                        0.0,
                        libm::sin(w.heading / 2.0) as f32,
                        0.0,
                        libm::cos(w.heading / 2.0) as f32,
                    ),
                    velocity,
                    archetype: (w.id % 7) as u16,
                    animation: u16::from(w.speed > 1.0) + u16::from(w.speed > 0.0),
                },
            ));
        }
    }
}

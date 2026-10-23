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

/// One person of a crowd that workers step (`forge_net::layer`, #268): everything its step reads is
/// its own, its turns drawn from its id and its tick, so whichever worker owns it steps it as any
/// other would. The same motion as [`Crowd`]'s: walking, strolling or standing, turning now and
/// then, turning back at the square's edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    /// Where, in world metres.
    pub position: DVec3,
    /// Its heading, radians from +z towards +x.
    pub heading: f64,
    /// Metres a second.
    pub speed: f64,
    /// Steps taken: what its turns are drawn from.
    pub tick: u64,
}

impl Crowd {
    /// Its people as workers step them ([`Person`]), by id: where and how [`Crowd::new`] made them.
    pub fn people(&self) -> Vec<(EntityId, Person)> {
        self.walkers
            .iter()
            .map(|w| {
                (
                    w.id,
                    Person {
                        position: w.position,
                        heading: w.heading,
                        speed: w.speed,
                        tick: 0,
                    },
                )
            })
            .collect()
    }
}

impl Person {
    /// Person `id` a step of `dt` seconds on, in a square `side` metres wide about the origin.
    pub fn step(&self, id: EntityId, dt: f64, side: f64) -> Person {
        let half = side / 2.0;
        let mut p = self.clone();
        p.tick += 1;
        if p.speed == 0.0 {
            return p;
        }
        let mut random =
            SplitMix64::new(u64::from(id) ^ p.tick.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if random.next_f64() < dt / 8.0 {
            p.heading += random.range_f64(-1.5, 1.5);
        }
        let (s, c) = (libm::sin(p.heading), libm::cos(p.heading));
        p.position.x += s * p.speed * dt;
        p.position.z += c * p.speed * dt;
        if p.position.x.abs() > half || p.position.z.abs() > half {
            p.heading += std::f64::consts::PI;
            p.position.x = p.position.x.clamp(-half, half);
            p.position.z = p.position.z.clamp(-half, half);
        }
        p
    }

    /// Its state as the replicator sends it ([`Crowd::states`]' form).
    pub fn state(&self, id: EntityId) -> EntityState {
        let (s, c) = (libm::sin(self.heading), libm::cos(self.heading));
        EntityState {
            position: self.position,
            rotation: Quat::from_xyzw(
                0.0,
                libm::sin(self.heading / 2.0) as f32,
                0.0,
                libm::cos(self.heading / 2.0) as f32,
            ),
            velocity: Vec3::new((s * self.speed) as f32, 0.0, (c * self.speed) as f32),
            archetype: (id % 7) as u16,
            animation: u16::from(self.speed > 1.0) + u16::from(self.speed > 0.0),
        }
    }
}

impl forge_sim::authority::Placed for Person {
    fn position(&self) -> DVec3 {
        self.position
    }
}

impl forge_sim::Codec for Person {
    fn encode(&self, out: &mut Vec<u8>) {
        for v in self.position.to_array() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.heading.to_le_bytes());
        out.extend_from_slice(&self.speed.to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
    }

    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        let mut v = [0_u64; 6];
        for x in &mut v {
            let (head, rest) = bytes.split_first_chunk::<8>()?;
            *x = u64::from_le_bytes(*head);
            *bytes = rest;
        }
        Some(Self {
            position: DVec3::new(
                f64::from_bits(v[0]),
                f64::from_bits(v[1]),
                f64::from_bits(v[2]),
            ),
            heading: f64::from_bits(v[3]),
            speed: f64::from_bits(v[4]),
            tick: v[5],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_sim::Codec;

    #[test]
    fn people_stay_in_the_square_and_cross_the_wire_unchanged() {
        let side = 60.0;
        let mut people = Crowd::new(200, side, 5).people();
        let start = people.clone();
        for _ in 0..3600 {
            for (id, p) in &mut people {
                *p = p.step(*id, 1.0 / 60.0, side);
            }
        }
        let half = side / 2.0;
        assert!(
            people
                .iter()
                .all(|(_, p)| p.position.x.abs() <= half && p.position.z.abs() <= half)
        );
        assert!(
            people
                .iter()
                .zip(&start)
                .filter(|(a, b)| a.1.position != b.1.position)
                .count()
                > 100
        );
        for (_, p) in &people {
            let mut bytes = Vec::new();
            p.encode(&mut bytes);
            assert_eq!(bytes.len(), 48);
            assert_eq!(Person::decode(&mut bytes.as_slice()).as_ref(), Some(p));
        }
    }
}

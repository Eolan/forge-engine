//! Quantisation: an entity's state in integers, the same bits on the server and its clients.
//!
//! The server quantises what it replicates and keeps the quantised state as the baseline it
//! sends deltas against; the client decodes the same integers, so both hold exactly the same
//! state (Fiedler, "State Synchronization": quantise on both sides). Positions are integer
//! steps (1 cm by default) of a world position in metres; orientations are smallest-three
//! quaternions (Fiedler, "Snapshot Compression": the largest component dropped and rebuilt from
//! the unit length, the other three in `±1/√2` at 9 bits each, 29 bits in all); velocities are
//! steps of 1/32 m/s. Only IEEE's correctly rounded operations are used (D-016): the same
//! inputs give the same integers on every machine.

use glam::{DVec3, Quat, Vec3};

/// An entity's state as the simulation holds it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityState {
    /// Where, in world metres (D-004's f64 frame).
    pub position: DVec3,
    /// Its orientation.
    pub rotation: Quat,
    /// Its velocity, metres a second.
    pub velocity: Vec3,
    /// What it is (a model, a kind of person): the client's to interpret.
    pub archetype: u16,
    /// What it does (an animation state, flags): the client's to interpret.
    pub animation: u16,
}

impl Default for EntityState {
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::ZERO,
            archetype: 0,
            animation: 0,
        }
    }
}

/// An entity's state in integers: what is sent and what deltas are taken against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct QState {
    /// Position steps on each axis.
    pub position: [i64; 3],
    /// The smallest-three orientation, packed: the dropped component's index in the top two
    /// bits, then the three others.
    pub rotation: u32,
    /// Velocity steps on each axis.
    pub velocity: [i32; 3],
    /// As [`EntityState::archetype`].
    pub archetype: u16,
    /// As [`EntityState::animation`].
    pub animation: u16,
}

/// The steps of quantisation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantization {
    /// Metres a position step.
    pub position_step: f64,
    /// A cell's side in position steps: a full position is sent as a cell and an offset in it.
    pub cell_steps: i64,
    /// Metres a second a velocity step.
    pub velocity_step: f32,
    /// Bits for each of the smallest three.
    pub rotation_bits: u32,
}

impl Default for Quantization {
    /// 1 cm, cells of 64 m (an offset of 13 bits), 1/32 m/s, 9 bits (D-010's walkers).
    fn default() -> Self {
        Self {
            position_step: 0.01,
            cell_steps: 6400,
            velocity_step: 1.0 / 32.0,
            rotation_bits: 9,
        }
    }
}

/// The largest velocity step: ±1024 m/s at 1/32 m/s fits a 16-bit class.
const VELOCITY_LIMIT: i32 = (1 << 15) - 1;

impl Quantization {
    /// Bits of an offset in a cell.
    pub fn offset_bits(&self) -> u32 {
        64 - ((self.cell_steps - 1) as u64).leading_zeros()
    }

    /// Bits of a packed orientation.
    pub fn rotation_total_bits(&self) -> u32 {
        2 + 3 * self.rotation_bits
    }

    /// A state in integers.
    pub fn quantize(&self, s: &EntityState) -> QState {
        let p = |v: f64| (v / self.position_step).round() as i64;
        let v = |x: f32| {
            ((x / self.velocity_step).round() as i32).clamp(-VELOCITY_LIMIT, VELOCITY_LIMIT)
        };
        QState {
            position: [p(s.position.x), p(s.position.y), p(s.position.z)],
            rotation: self.pack_rotation(s.rotation),
            velocity: [v(s.velocity.x), v(s.velocity.y), v(s.velocity.z)],
            archetype: s.archetype,
            animation: s.animation,
        }
    }

    /// The state integers stand for.
    pub fn dequantize(&self, q: &QState) -> EntityState {
        let p = |v: i64| v as f64 * self.position_step;
        let v = |x: i32| x as f32 * self.velocity_step;
        EntityState {
            position: DVec3::new(p(q.position[0]), p(q.position[1]), p(q.position[2])),
            rotation: self.unpack_rotation(q.rotation),
            velocity: Vec3::new(v(q.velocity[0]), v(q.velocity[1]), v(q.velocity[2])),
            archetype: q.archetype,
            animation: q.animation,
        }
    }

    /// Smallest three: the largest component dropped (its sign made positive), the others
    /// mapped from `±1/√2` to `0..2^bits`.
    pub fn pack_rotation(&self, q: Quat) -> u32 {
        let q = q.normalize();
        let c = [q.x, q.y, q.z, q.w];
        let mut largest = 0;
        for k in 1..4 {
            if c[k].abs() > c[largest].abs() {
                largest = k;
            }
        }
        let sign = if c[largest] < 0.0 { -1.0 } else { 1.0 };
        let max = ((1u32 << self.rotation_bits) - 1) as f32;
        let mut packed = largest as u32;
        for (k, &x) in c.iter().enumerate() {
            if k == largest {
                continue;
            }
            let unit = (x * sign * std::f32::consts::SQRT_2 + 1.0) * 0.5;
            let step = (unit.clamp(0.0, 1.0) * max).round() as u32;
            packed = packed << self.rotation_bits | step;
        }
        packed
    }

    /// The orientation [`Quantization::pack_rotation`] packed.
    pub fn unpack_rotation(&self, packed: u32) -> Quat {
        let bits = self.rotation_bits;
        let max = ((1u32 << bits) - 1) as f32;
        let mask = (1u32 << bits) - 1;
        let largest = (packed >> (3 * bits)) as usize & 3;
        let mut three = [0.0f32; 3];
        for (k, t) in three.iter_mut().enumerate() {
            let step = packed >> (bits * (2 - k as u32)) & mask;
            *t = (step as f32 / max * 2.0 - 1.0) * std::f32::consts::FRAC_1_SQRT_2;
        }
        let rest = (1.0 - three.iter().map(|t| t * t).sum::<f32>())
            .max(0.0)
            .sqrt();
        let mut c = [0.0f32; 4];
        let mut next = three.iter();
        for (k, x) in c.iter_mut().enumerate() {
            *x = if k == largest {
                rest
            } else {
                *next.next().expect("three")
            };
        }
        Quat::from_xyzw(c[0], c[1], c[2], c[3]).normalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::SplitMix64;

    #[test]
    fn positions_and_velocities_round_to_half_a_step() {
        let q = Quantization::default();
        let mut random = SplitMix64::new(4);
        for _ in 0..10_000 {
            let s = EntityState {
                position: DVec3::new(
                    random.range_f64(-1e6, 1e6),
                    random.range_f64(-100.0, 400.0),
                    random.range_f64(-1e6, 1e6),
                ),
                velocity: Vec3::new(
                    random.range_f32(-50.0, 50.0),
                    random.range_f32(-5.0, 5.0),
                    random.range_f32(-50.0, 50.0),
                ),
                ..EntityState::default()
            };
            let back = q.dequantize(&q.quantize(&s));
            assert!((back.position - s.position).abs().max_element() <= 0.005 + 1e-9);
            assert!((back.velocity - s.velocity).abs().max_element() <= 1.0 / 64.0 + 1e-6);
            assert_eq!(
                q.quantize(&back),
                q.quantize(&s),
                "quantising again changes nothing"
            );
        }
        assert_eq!(q.offset_bits(), 13);
    }

    #[test]
    fn smallest_three_keeps_orientations_within_a_fraction_of_a_degree() {
        let q = Quantization::default();
        assert_eq!(q.rotation_total_bits(), 29);
        let mut random = SplitMix64::new(5);
        let mut worst = 0.0f32;
        for _ in 0..10_000 {
            let r = Quat::from_xyzw(
                random.range_f32(-1.0, 1.0),
                random.range_f32(-1.0, 1.0),
                random.range_f32(-1.0, 1.0),
                random.range_f32(-1.0, 1.0),
            )
            .normalize();
            let back = q.unpack_rotation(q.pack_rotation(r));
            worst = worst.max(r.angle_between(back).to_degrees());
        }
        assert!(worst < 0.5, "worst error {worst}°");
    }
}

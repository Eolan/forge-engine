//! The shore's waves (issue #105, D-038's shore; `docs/research/water.md` §2 and its
//! recommendation's step 2): trains of waves that come in along the coast distance's gradient.
//! Over the shallowing floor they slow and shorten (the dispersion relation), grow as their
//! energy flux crowds into slower water (shoaling), and break once they stand higher than
//! [`BREAKER_INDEX`] of the depth. The GPU draws them as a function of position and time
//! (`water.slang`); what it cannot integrate per pixel, the time a crest takes from a point to
//! the shore, is tabulated here against the coast distance, from the floor's mean depth at
//! each distance (the island's sea floor is a function of that distance, `sea_floor`).

use forge_core::dmath::{exp, tanh};

use crate::field::Field2;

/// Gravity, m/s².
pub const GRAVITY: f64 = 9.81;

/// The breaker index γ: a wave breaks once its height passes γ times the depth (McCowan 1894's
/// limit for the highest solitary wave, the usual value for spilling breakers on gentle
/// beaches).
pub const BREAKER_INDEX: f64 = 0.78;

/// The largest shoaling coefficient a table holds: the coefficient grows without bound as the
/// depth goes to zero, and the breaker index caps the height long before.
const SHOALING_CAP: f64 = 8.0;

/// The wave number `k`, rad/m, of angular frequency `omega` in `depth` metres of water: the
/// root of `ω² = g k tanh(k h)`, by Newton's method from the shallow- and deep-water limits.
pub fn wave_number(omega: f64, depth: f64) -> f64 {
    if depth <= 0.0 {
        return f64::INFINITY;
    }
    let deep = omega * omega / GRAVITY;
    let shallow = omega / (GRAVITY * depth).sqrt();
    let mut k = deep.max(shallow);
    for _ in 0..32 {
        let t = tanh(k * depth);
        let f = GRAVITY * k * t - omega * omega;
        let df = GRAVITY * (t + k * depth * (1.0 - t * t));
        let step = f / df;
        k -= step;
        if step.abs() <= 1e-12 * k {
            break;
        }
    }
    k
}

/// The group velocity's share of the phase velocity at `kh`: `½ (1 + 2kh / sinh 2kh)`, from 1
/// in shallow water to ½ in deep water.
fn group_share(kh: f64) -> f64 {
    let x = 2.0 * kh;
    if x > 40.0 {
        return 0.5;
    }
    if x < 1e-6 {
        return 1.0;
    }
    let sinh = 0.5 * (exp(x) - exp(-x));
    0.5 * (1.0 + x / sinh)
}

/// The sea floor's mean depth against the distance to the coast, in bins of `bin` metres out
/// from the shore.
#[derive(Clone, Debug, PartialEq)]
pub struct ShoreProfile {
    /// Metres a bin.
    pub bin: f64,
    /// Per bin, the floor's depth at its centre, metres: bin `j` covers the distances from
    /// `j × bin` to `(j + 1) × bin` out from the shore.
    pub depth: Vec<f64>,
}

impl ShoreProfile {
    /// The profile of the samples of `floor` below `sea_level`, binned by their distance to the
    /// coast (`coast`, negative at sea: `coast_distance`), `bins` bins of `bin` metres. Each
    /// filled bin gives a knot, its samples' mean distance and mean depth; the shore is a knot
    /// at no depth; the bins' centres read the knots linearly (beyond the last, its depth).
    pub fn new(
        floor: &Field2<f32>,
        coast: &Field2<f32>,
        sea_level: f32,
        bin: f64,
        bins: usize,
    ) -> Self {
        assert_eq!(floor.size, coast.size, "the coast distance of this floor");
        let mut sums = vec![(0.0_f64, 0.0_f64, 0_u32); bins];
        for (&h, &d) in floor.data.iter().zip(&coast.data) {
            if h >= sea_level || d >= 0.0 {
                continue;
            }
            let distance = f64::from(-d);
            let j = (distance / bin) as usize;
            if j < bins {
                let s = &mut sums[j];
                s.0 += distance;
                s.1 += f64::from(sea_level - h);
                s.2 += 1;
            }
        }
        let knots: Vec<(f64, f64)> = std::iter::once((0.0, 0.0))
            .chain(
                sums.iter()
                    .filter(|s| s.2 > 0)
                    .map(|&(d, h, c)| (d / f64::from(c), h / f64::from(c))),
            )
            .collect();
        let mut k = 0;
        let depth = (0..bins)
            .map(|j| {
                let centre = (j as f64 + 0.5) * bin;
                while k + 1 < knots.len() && knots[k + 1].0 < centre {
                    k += 1;
                }
                match knots.get(k + 1) {
                    Some(&(d1, h1)) => {
                        let (d0, h0) = knots[k];
                        h0 + (h1 - h0) * ((centre - d0) / (d1 - d0)).clamp(0.0, 1.0)
                    }
                    None => knots[k].1,
                }
            })
            .collect();
        Self { bin, depth }
    }

    /// The mean depth `distance` metres out from the shore: the bins' values at their centres,
    /// linear between them, from zero at the shore; beyond the last bin, its value.
    pub fn depth_at(&self, distance: f64) -> f64 {
        if distance <= 0.0 || self.depth.is_empty() {
            return 0.0;
        }
        let x = distance / self.bin - 0.5;
        if x <= 0.0 {
            // Between the shore and the first bin's centre.
            return self.depth[0] * distance / (0.5 * self.bin);
        }
        let j = x as usize;
        if j + 1 >= self.depth.len() {
            return *self.depth.last().unwrap_or(&0.0);
        }
        let f = x - j as f64;
        self.depth[j] * (1.0 - f) + self.depth[j + 1] * f
    }
}

/// A train of waves arriving at the shore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShoreTrain {
    /// Seconds from crest to crest.
    pub period: f64,
    /// The height in deep water, crest to trough, metres.
    pub height: f64,
}

impl ShoreTrain {
    /// Radians a second.
    pub fn omega(&self) -> f64 {
        std::f64::consts::TAU / self.period
    }

    /// Per bin boundary out from the shore (0, `bin`, 2 `bin`, … for as many boundaries as the
    /// profile has bins): the time a crest takes from there to the shore, seconds, and the
    /// shoaling coefficient there, `√(cg₀ / cg)`, capped at 8. The time sums `2 bin / (c₀ + c₁)`
    /// over the bins, `c` the phase speed at their ends: exact for a floor sloping evenly in
    /// shallow water, where the speed goes to zero at the shore.
    pub fn table(&self, profile: &ShoreProfile) -> Vec<[f32; 2]> {
        let omega = self.omega();
        let deep_group = 0.5 * GRAVITY / omega;
        let mut time = 0.0;
        let mut speed = 0.0;
        (0..profile.depth.len())
            .map(|j| {
                let depth = profile.depth_at(j as f64 * profile.bin);
                let (next_speed, shoaling) = if depth > 0.0 {
                    let k = wave_number(omega, depth);
                    let group = group_share(k * depth) * omega / k;
                    (omega / k, (deep_group / group).sqrt().min(SHOALING_CAP))
                } else {
                    (0.0, SHOALING_CAP)
                };
                if j > 0 {
                    time += 2.0 * profile.bin / (speed + next_speed);
                }
                speed = next_speed;
                [time as f32, shoaling as f32]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wave_number_meets_both_limits_and_the_dispersion_relation() {
        let omega = std::f64::consts::TAU / 8.0;
        // Deep water: k = ω²/g, a wavelength of g T² / 2π (about 100 m for 8 s).
        let deep = wave_number(omega, 1000.0);
        assert!((deep / (omega * omega / GRAVITY) - 1.0).abs() < 1e-9);
        assert!((std::f64::consts::TAU / deep - 99.9).abs() < 0.2);
        // Shallow water: k = ω / √(g h), within 2 % at h = λ/40.
        let shallow = wave_number(omega, 0.5);
        assert!((shallow / (omega / (GRAVITY * 0.5).sqrt()) - 1.0).abs() < 0.02);
        for depth in [0.2, 2.0, 10.0, 40.0] {
            let k = wave_number(omega, depth);
            let residual = GRAVITY * k * tanh(k * depth) - omega * omega;
            assert!(residual.abs() < 1e-9, "{depth}: {residual}");
        }
        assert_eq!(wave_number(omega, 0.0), f64::INFINITY);
    }

    #[test]
    fn a_beach_profile_slows_the_crests_and_grows_them_until_the_shallows() {
        // A 4 % beach out to 60 m, the floor a function of the distance, as the island's is.
        let n = 257;
        let spacing = 8.0;
        let coast = Field2::from_fn(n, spacing, |x, _| (100.0 - x as f32) * spacing as f32);
        let floor = Field2::from_fn(n, spacing, |x, _| {
            let d = (100.0 - x as f32) * spacing as f32;
            if d > 0.0 {
                d * 0.01
            } else {
                (0.04 * d).max(-60.0)
            }
        });
        let profile = ShoreProfile::new(&floor, &coast, 0.0, 4.0, 256);
        assert!(profile.depth_at(0.0) == 0.0);
        assert!(
            (profile.depth_at(100.0) - 4.0).abs() < 0.3,
            "{}",
            profile.depth_at(100.0)
        );
        assert!((profile.depth_at(900.0) - 36.0).abs() < 0.5);
        let train = ShoreTrain {
            period: 8.0,
            height: 1.0,
        };
        let table = train.table(&profile);
        assert_eq!(table.len(), 256);
        assert_eq!(table[0][0], 0.0);
        // The time grows outwards, faster near the shore where the crests are slow.
        for pair in table.windows(2) {
            assert!(pair[1][0] > pair[0][0]);
        }
        let near = table[5][0] - table[4][0];
        let far = table[200][0] - table[199][0];
        assert!(near > 2.0 * far, "{near} s against {far} s a bin");
        // In shallow water the time to the shore is 2 √(d / (g s)) on a slope s.
        let shallow = 2.0 * (20.0 / (GRAVITY * 0.04)).sqrt();
        assert!((f64::from(table[5][0]) / shallow - 1.0).abs() < 0.1);
        // Shoaling: under 1 at intermediate depths, above it in the shallows, capped at the shore.
        assert!(table[50][1] < 1.0 && table[2][1] > 1.2);
        assert_eq!(table[0][1], 8.0);
    }
}

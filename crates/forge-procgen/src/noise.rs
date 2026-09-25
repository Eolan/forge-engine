//! Gradient noise on an integer lattice, deterministic on every machine: the gradient of each
//! lattice point is one of eight fixed directions chosen by `forge_core::hash::hash_cell2`, the
//! blend a quintic fade, so a sample needs no transcendental function and the same seed gives
//! the same bits on a client and a server (D-016). Values lie in about `[−1, 1]`.

use forge_core::hash::hash_cell2;

/// The eight gradient directions (the diagonals scaled to unit length).
const GRADIENTS: [(f64, f64); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ),
    (
        -std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
    ),
    (
        std::f64::consts::FRAC_1_SQRT_2,
        -std::f64::consts::FRAC_1_SQRT_2,
    ),
    (
        -std::f64::consts::FRAC_1_SQRT_2,
        -std::f64::consts::FRAC_1_SQRT_2,
    ),
];

/// The gradient at lattice point (x, y) for `seed`.
#[inline]
fn gradient(seed: u64, x: i32, y: i32) -> (f64, f64) {
    GRADIENTS[(hash_cell2(seed, x, y) & 7) as usize]
}

/// Perlin's quintic fade, `6t⁵ − 15t⁴ + 10t³`.
#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Gradient noise at (x, y), lattice points at the integers, in about `[−1, 1]`.
pub fn gradient2(seed: u64, x: f64, y: f64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);
    let dot = |dx: i32, dy: i32| {
        let (gx, gy) = gradient(seed, ix + dx, iy + dy);
        gx * (fx - f64::from(dx)) + gy * (fy - f64::from(dy))
    };
    let (u, v) = (fade(fx), fade(fy));
    let top = dot(0, 0) + (dot(1, 0) - dot(0, 0)) * u;
    let bottom = dot(0, 1) + (dot(1, 1) - dot(0, 1)) * u;
    // Gradient noise peaks at ±√2/2 on this lattice; scaled to about ±1.
    (top + (bottom - top) * v) * std::f64::consts::SQRT_2
}

/// Fractal sum of `octaves` of [`gradient2`], each `lacunarity` times finer and `gain` times
/// weaker, normalised to about `[−1, 1]`.
pub fn fbm(seed: u64, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    let (mut sum, mut amplitude, mut frequency, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for octave in 0..octaves {
        sum += amplitude
            * gradient2(
                seed ^ (u64::from(octave) * 0x9E37_79B9_7F4A_7C15),
                x * frequency,
                y * frequency,
            );
        norm += amplitude;
        amplitude *= gain;
        frequency *= lacunarity;
    }
    sum / norm
}

/// Ridged fractal noise: `1 − |noise|` per octave, sharp crests, in `[0, 1]`.
pub fn ridged(seed: u64, x: f64, y: f64, octaves: u32, lacunarity: f64, gain: f64) -> f64 {
    let (mut sum, mut amplitude, mut frequency, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for octave in 0..octaves {
        let n = gradient2(
            seed ^ (u64::from(octave) * 0x9E37_79B9_7F4A_7C15),
            x * frequency,
            y * frequency,
        );
        let ridge = 1.0 - n.abs().min(1.0);
        sum += amplitude * ridge * ridge;
        norm += amplitude;
        amplitude *= gain;
        frequency *= lacunarity;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_zero_on_the_lattice_bounded_between_and_the_same_every_time() {
        assert_eq!(gradient2(7, 3.0, -2.0), 0.0);
        let mut lo = f64::MAX;
        let mut hi = f64::MIN;
        for i in 0..200 {
            for j in 0..200 {
                let v = gradient2(7, f64::from(i) * 0.137 + 0.5, f64::from(j) * 0.093);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        assert!(
            lo < -0.5 && hi > 0.5 && lo >= -1.01 && hi <= 1.01,
            "{lo} {hi}"
        );
        assert_eq!(fbm(7, 1.5, 2.5, 4, 2.0, 0.5), fbm(7, 1.5, 2.5, 4, 2.0, 0.5));
        assert_ne!(fbm(7, 1.5, 2.5, 4, 2.0, 0.5), fbm(8, 1.5, 2.5, 4, 2.0, 0.5));
        let r = ridged(7, 1.3, 0.7, 3, 2.0, 0.5);
        assert!((0.0..=1.0).contains(&r));
    }
}

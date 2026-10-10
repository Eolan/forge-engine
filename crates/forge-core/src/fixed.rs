//! Fixed-point arithmetic for what the CPU, the GPU and a server must agree on to the bit
//! (D-016's amendment of 2026-10-10, D-058's step 2): `i64` values with a fixed number of
//! fraction bits, products through 128 bits, and the trigonometry by CORDIC (shifts and adds).
//! Every operation is exact in Rust's and in Slang's integers alike; `shaders/fixed.slang` is
//! the same code, its 128-bit products made of four 32-bit ones.
//!
//! Two formats carry most of it:
//! - **Q61** ([`ONE`]): angles in radians, sines and cosines, unit directions (±4 at most).
//! - **Q32** ([`METRE`]): metres, for positions and heights on a planet (±2.1 million km in
//!   steps of 0.23 nm).
//!
//! The CORDIC's angle table and gain are worked out once ([`Cordic::new`]) from `libm` (the
//! same on every machine, D-016) and handed to the GPU as data, so both sides read the same
//! integers.

/// One in Q61.
pub const ONE: i64 = 1 << 61;
/// Fraction bits of Q61.
pub const Q61: u32 = 61;
/// One metre in Q32.
pub const METRE: i64 = 1 << 32;
/// Fraction bits of Q32.
pub const Q32: u32 = 32;

/// CORDIC iterations: about one bit each, 2⁻⁵⁶ rad at the end.
pub const CORDIC_STEPS: usize = 56;

/// `a × b >> shift`, rounded toward −∞, through 128 bits, wrapped to 64 (`shift` < 64).
#[inline]
#[must_use]
pub fn mul_shr(a: i64, b: i64, shift: u32) -> i64 {
    ((i128::from(a) * i128::from(b)) >> shift) as i64
}

/// `a × b` in Q61.
#[inline]
#[must_use]
pub fn mul61(a: i64, b: i64) -> i64 {
    mul_shr(a, b, Q61)
}

/// `a × b` in Q32.
#[inline]
#[must_use]
pub fn mul32(a: i64, b: i64) -> i64 {
    mul_shr(a, b, Q32)
}

/// `value` (a real number) in Q`bits`, rounded to nearest: the constants made once on the CPU
/// (`libm`, the same on every machine).
#[must_use]
pub fn fixed(value: f64, bits: u32) -> i64 {
    libm::round(libm::ldexp(value, bits as i32)) as i64
}

/// A Q`bits` value as a real number.
#[must_use]
pub fn to_f64(value: i64, bits: u32) -> f64 {
    value as f64 / (2.0_f64).powi(bits as i32)
}

/// The CORDIC's constants: the angles `atan 2⁻ᵏ` in Q61, and `1 / K`, the inverse of its gain.
#[derive(Clone, Debug)]
pub struct Cordic {
    /// `atan(2⁻ᵏ)` in Q61, for k from 0.
    pub angles: [i64; CORDIC_STEPS],
    /// `∏ 1 / √(1 + 2⁻²ᵏ)` in Q61.
    pub inverse_gain: i64,
    /// Its gain `K = ∏ √(1 + 2⁻²ᵏ)` in Q61: what vectoring multiplies a length by.
    pub gain: i64,
    /// π / 2 in Q61.
    pub half_pi: i64,
    /// π in Q61.
    pub pi: i64,
}

impl Cordic {
    /// Worked out from `libm`, identical on every machine.
    #[must_use]
    pub fn new() -> Self {
        let mut angles = [0; CORDIC_STEPS];
        let mut gain = 1.0_f64;
        for (k, angle) in angles.iter_mut().enumerate() {
            let t = libm::ldexp(1.0, -(k as i32));
            *angle = q61(libm::atan(t));
            gain *= libm::sqrt(1.0 + t * t);
        }
        Self {
            angles,
            inverse_gain: q61(1.0 / gain),
            gain: q61(gain),
            half_pi: q61(std::f64::consts::FRAC_PI_2),
            pi: q61(std::f64::consts::PI),
        }
    }

    /// `(cos θ, sin θ)` in Q61 for `theta` in Q61, |θ| ≤ π/2 (rotation mode).
    #[must_use]
    pub fn cos_sin(&self, theta: i64) -> (i64, i64) {
        let (mut x, mut y, mut z) = (self.inverse_gain, 0_i64, theta);
        for (k, &angle) in self.angles.iter().enumerate() {
            let (dx, dy) = (y >> k, x >> k);
            if z >= 0 {
                x -= dx;
                y += dy;
                z -= angle;
            } else {
                x += dx;
                y -= dy;
                z += angle;
            }
        }
        (x, y)
    }

    /// `atan2(y, x)` in Q61 for `y`, `x` in Q61 (|·| ≤ 2), and `K √(x² + y²)` (vectoring mode,
    /// after a quarter turn when `x` is negative).
    #[must_use]
    pub fn atan2(&self, y: i64, x: i64) -> (i64, i64) {
        let (mut x, mut y, mut z) = if x >= 0 {
            (x, y, 0)
        } else if y >= 0 {
            (y, -x, self.half_pi)
        } else {
            (-y, x, -self.half_pi)
        };
        for (k, &angle) in self.angles.iter().enumerate() {
            let (dx, dy) = (y >> k, x >> k);
            if y >= 0 {
                x += dx;
                y -= dy;
                z += angle;
            } else {
                x -= dx;
                y += dy;
                z -= angle;
            }
        }
        (z, x)
    }
}

impl Default for Cordic {
    fn default() -> Self {
        Self::new()
    }
}

/// `value` in Q61, rounded to nearest.
fn q61(value: f64) -> i64 {
    fixed(value, Q61)
}

/// `1 / √r2` in Q61 for `r2` in Q61, 1/4 ≤ r2 ≤ 2: Newton's iteration from 1, eight steps (it
/// diverges from 1 past r2 = 3).
#[must_use]
pub fn rsqrt61(r2: i64) -> i64 {
    // y ← y (3 − r2 y²) / 2, quadratic from within a factor of two.
    let mut y = ONE;
    for _ in 0..8 {
        let three = 3 * ONE - mul61(r2, mul61(y, y));
        y = mul61(y, three) >> 1;
    }
    y
}

/// Perlin's quintic fade `6t⁵ − 15t⁴ + 10t³` in Q32 for `t` in Q32, 0 ≤ t < 1.
#[must_use]
pub fn fade32(t: i64) -> i64 {
    let inner = mul32(t, 6 * METRE) - 15 * METRE;
    let inner = mul32(t, inner) + 10 * METRE;
    mul32(mul32(mul32(t, t), t), inner)
}

/// `t` clamped to 0..1 and eased (`3t² − 2t³`), Q32.
#[must_use]
pub fn smooth32(t: i64) -> i64 {
    let t = t.clamp(0, METRE);
    mul32(mul32(t, t), 3 * METRE - 2 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cordic_s_sines_and_angles_hold_to_its_steps() {
        let cordic = Cordic::new();
        for i in -50..=50 {
            let theta = f64::from(i) * 0.031;
            let (c, s) = cordic.cos_sin(q61(theta));
            assert!((to_f64(c, Q61) - theta.cos()).abs() < 1e-15, "cos {theta}");
            assert!((to_f64(s, Q61) - theta.sin()).abs() < 1e-15, "sin {theta}");
        }
        for (y, x) in [
            (0.3, 0.9),
            (-0.7, 0.2),
            (0.5, -0.5),
            (-0.1, -0.99),
            (1.0, 0.0),
            (0.0, -1.0),
        ] {
            let (angle, length) = cordic.atan2(q61(y), q61(x));
            let want = f64::atan2(y, x);
            assert!((to_f64(angle, Q61) - want).abs() < 1e-15, "atan2({y}, {x})");
            let gain = 1.0 / to_f64(cordic.inverse_gain, Q61);
            assert!((to_f64(length, Q61) / gain - f64::hypot(x, y)).abs() < 1e-14);
        }
    }

    #[test]
    fn the_inverse_square_root_and_the_fades_hold() {
        for r2 in [0.26, 1.0 / 3.0, 0.5, 1.0, 1.7] {
            let y = to_f64(rsqrt61(q61(r2)), Q61);
            assert!((y - 1.0 / f64::sqrt(r2)).abs() < 1e-15, "{r2}");
        }
        for t in [0.0, 0.1, 0.5, 0.77, 0.999] {
            let f = to_f64(fade32((t * METRE as f64) as i64), Q32);
            let want = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
            assert!((f - want).abs() < 1e-8, "{t}");
        }
    }

    #[test]
    fn products_round_down_through_128_bits() {
        assert_eq!(mul61(ONE, ONE), ONE);
        assert_eq!(mul61(-1, 1), -1);
        assert_eq!(mul32(3 * METRE, -METRE / 2), -3 * METRE / 2);
        assert_eq!(mul_shr(i64::MAX, 2, 1), i64::MAX);
    }
}

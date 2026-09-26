//! The open sea's waves on the CPU: a directional wave spectrum (JONSWAP, Hasselmann et al.
//! 1973, with the TMA finite-depth factor of Bouws et al. 1985 and Horvath's 2015 spreading
//! with a swell parameter) synthesised by an inverse FFT into a tiling patch of heights,
//! horizontal displacements, slopes and the Jacobian that marks the whitecaps (Tessendorf
//! 2001). This is the CPU side of `docs/research/water.md`'s recommendation and of D-009's
//! "spectrum evaluated identically on CPU and GPU": the spectrum's amplitudes and phases are a
//! pure function of the seed, every transcendental goes through `forge_core::dmath`, and the
//! transform's order of operations is fixed, so the surface is the same bytes on every
//! machine (D-016). The GPU's version (`ocean.slang`, later) is diffed against this one.

use std::time::Instant;

use forge_core::Seed;
use forge_core::dmath::{exp, ln, powf, sin_cos, tanh};

use crate::field::Field2;

/// Gravity, m/s².
const G: f64 = 9.81;

/// What the sea is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanParams {
    /// Every random phase derives from it.
    pub seed: Seed,
    /// Samples per side of the patch, a power of two (256).
    pub size: u32,
    /// The patch's side, metres; the waves tile with this period.
    pub patch: f64,
    /// The wind at 10 m, m/s.
    pub wind_speed: f64,
    /// The wind's direction, radians from +x towards +y.
    pub wind_direction: f64,
    /// The fetch the wind has blown over, metres (JONSWAP's peak and energy come from it).
    pub fetch: f64,
    /// The water's depth, metres (`f64::INFINITY` for deep water: no TMA damping).
    pub depth: f64,
    /// Horvath's swell parameter, 0 (a young, wide sea) to 1 (a narrow swell).
    pub swell: f64,
    /// The horizontal displacement's scale (Tessendorf's choppiness), 0 for none.
    pub choppiness: f64,
    /// Waves shorter than this, metres, are left out (they are the next cascade's).
    pub shortest_wave: f64,
}

impl OceanParams {
    /// A fresh breeze over a long fetch onto a 50 m shelf, the 256 m patch at a metre: the
    /// island's middle cascade.
    pub fn breeze(seed: Seed) -> Self {
        Self {
            seed,
            size: 256,
            patch: 256.0,
            wind_speed: 12.0,
            wind_direction: 0.6,
            fetch: 200_000.0,
            depth: 50.0,
            swell: 0.3,
            choppiness: 1.0,
            shortest_wave: 2.0,
        }
    }
}

/// The dispersion relation in water `depth` deep: `ω` and `dω/dk` at wave number `k`.
fn dispersion(k: f64, depth: f64) -> (f64, f64) {
    if depth.is_infinite() {
        let omega = (G * k).sqrt();
        return (omega, 0.5 * G / omega);
    }
    let th = tanh(k * depth);
    let omega = (G * k * th).sqrt();
    let sech2 = 1.0 - th * th;
    (omega, G * (th + k * depth * sech2) / (2.0 * omega))
}

/// JONSWAP's frequency spectrum `S(ω)`, m²·s, for the wind and fetch, and its peak `ωp`.
fn jonswap(omega: f64, wind_speed: f64, fetch: f64) -> (f64, f64) {
    let dimensionless = G * fetch / (wind_speed * wind_speed);
    let alpha = 0.076 * powf(dimensionless, -0.22);
    let omega_p = 22.0 * (G / wind_speed) * powf(dimensionless, -0.33);
    if omega <= 0.0 {
        return (0.0, omega_p);
    }
    let sigma = if omega <= omega_p { 0.07 } else { 0.09 };
    let r = exp(-(omega - omega_p) * (omega - omega_p) / (2.0 * sigma * sigma * omega_p * omega_p));
    let ratio = omega_p / omega;
    let shape = alpha * G * G / powf(omega, 5.0) * exp(-1.25 * ratio * ratio * ratio * ratio);
    (shape * powf(3.3, r), omega_p)
}

/// The TMA factor of Kitaigorodskii (Bouws et al. 1985): how a finite depth caps the spectrum.
fn tma(omega: f64, depth: f64) -> f64 {
    if depth.is_infinite() {
        return 1.0;
    }
    let wh = omega * (depth / G).sqrt();
    if wh <= 1.0 {
        0.5 * wh * wh
    } else if wh < 2.0 {
        1.0 - 0.5 * (2.0 - wh) * (2.0 - wh)
    } else {
        1.0
    }
}

/// Hasselmann's spreading exponent with Horvath's swell term.
fn spreading_exponent(omega: f64, omega_p: f64, wind_speed: f64, swell: f64) -> f64 {
    let s = if omega <= omega_p {
        6.97 * powf(omega / omega_p, 4.06)
    } else {
        9.77 * powf(
            omega / omega_p,
            -2.33 - 1.45 * (wind_speed * omega_p / G - 1.17),
        )
    };
    s + 16.0 * tanh(omega_p / omega) * swell * swell
}

/// `cos^{2s}(θ/2)` normalised over the circle by a fixed quadrature (the same on every
/// machine), so the directional spectrum integrates to the frequency spectrum.
fn spreading(theta: f64, s: f64) -> f64 {
    const SAMPLES: usize = 64;
    let lobe = |t: f64| {
        let (_, c) = sin_cos(0.5 * t);
        if c <= 0.0 { 0.0 } else { powf(c, 2.0 * s) }
    };
    let step = std::f64::consts::TAU / SAMPLES as f64;
    let total: f64 = (0..SAMPLES)
        .map(|k| lobe(-std::f64::consts::PI + (k as f64 + 0.5) * step) * step)
        .sum();
    if total > 0.0 {
        lobe(theta) / total
    } else {
        0.0
    }
}

/// One complex number.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct C(f64, f64);

impl C {
    fn mul(self, o: C) -> C {
        C(self.0 * o.0 - self.1 * o.1, self.0 * o.1 + self.1 * o.0)
    }
    fn add(self, o: C) -> C {
        C(self.0 + o.0, self.1 + o.1)
    }
    fn conj(self) -> C {
        C(self.0, -self.1)
    }
    fn scale(self, s: f64) -> C {
        C(self.0 * s, self.1 * s)
    }
    /// `e^{iφ}`.
    fn phasor(phi: f64) -> C {
        let (s, c) = sin_cos(phi);
        C(c, s)
    }
}

/// The in-place inverse FFT of `data` (length a power of two): radix-2, decimation in time,
/// the twiddles from `dmath`, unnormalised.
fn ifft(data: &mut [C], twiddles: &[C]) {
    let n = data.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            data.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let stride = n / len;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let w = twiddles[k * stride];
                let a = data[start + k];
                let b = data[start + k + len / 2].mul(w);
                data[start + k] = a.add(b);
                data[start + k + len / 2] = a.add(b.scale(-1.0));
            }
        }
        len <<= 1;
    }
}

/// The 2-D inverse FFT of `grid` (`n × n`, row-major), rows then columns, as a plain sum
/// `Σ_k h(k) e^{ik·x}` (Tessendorf's synthesis: the amplitudes already carry the energy).
fn ifft2(grid: &mut [C], n: usize) {
    let twiddles: Vec<C> = (0..n / 2)
        .map(|k| C::phasor(std::f64::consts::TAU * k as f64 / n as f64))
        .collect();
    for row in grid.chunks_mut(n) {
        ifft(row, &twiddles);
    }
    let mut column = vec![C::default(); n];
    for x in 0..n {
        for (y, c) in column.iter_mut().enumerate() {
            *c = grid[y * n + x];
        }
        ifft(&mut column, &twiddles);
        for (y, c) in column.iter().enumerate() {
            grid[y * n + x] = *c;
        }
    }
}

/// The sea's spectrum: the initial amplitudes `h0(k)` and the frequencies, from which any
/// instant's surface follows.
#[derive(Clone, Debug, PartialEq)]
pub struct Ocean {
    /// Its parameters.
    pub params: OceanParams,
    /// The wave vector per sample, `(kx, ky)`, rad/m.
    k: Vec<(f64, f64)>,
    /// `h0(k)` per sample.
    h0: Vec<C>,
    /// `ω(k)` per sample.
    omega: Vec<f64>,
}

/// The surface at one instant: heights, horizontal displacements, slopes and the Jacobian,
/// each a tiling field of `params.size²` samples `params.patch / params.size` metres apart.
#[derive(Clone, Debug, PartialEq)]
pub struct OceanSurface {
    /// The height above the mean sea, metres.
    pub height: Field2<f32>,
    /// The displacement along +x, metres (choppiness applied).
    pub dx: Field2<f32>,
    /// The displacement along +y, metres.
    pub dy: Field2<f32>,
    /// `∂h/∂x`.
    pub slope_x: Field2<f32>,
    /// `∂h/∂y`.
    pub slope_y: Field2<f32>,
    /// The Jacobian of the displaced surface: below zero the wave folds over, which is foam.
    pub jacobian: Field2<f32>,
}

impl Ocean {
    /// The spectrum for `params`: Gaussian amplitudes from the seed on every wave vector of
    /// the patch, scaled by the directional spectrum's energy in that cell of k-space.
    pub fn new(params: OceanParams) -> Self {
        let n = params.size as usize;
        assert!(
            n.is_power_of_two() && n >= 2,
            "the patch is a power of two a side"
        );
        let dk = std::f64::consts::TAU / params.patch;
        let count = n * n;
        let mut k = Vec::with_capacity(count);
        let mut h0 = Vec::with_capacity(count);
        let mut omega = Vec::with_capacity(count);
        let k_max = std::f64::consts::TAU / params.shortest_wave;
        let (_, omega_p) = jonswap(1.0, params.wind_speed, params.fetch);
        for y in 0..n {
            for x in 0..n {
                // Frequencies in FFT order: 0..n/2 positive, then negative.
                let m = |i: usize| {
                    if i < n / 2 {
                        i as f64
                    } else {
                        i as f64 - n as f64
                    }
                };
                let (kx, ky) = (m(x) * dk, m(y) * dk);
                let kk = (kx * kx + ky * ky).sqrt();
                k.push((kx, ky));
                if kk == 0.0 || kk > k_max {
                    h0.push(C::default());
                    omega.push(0.0);
                    continue;
                }
                let (w, dw_dk) = dispersion(kk, params.depth);
                let (s_omega, _) = jonswap(w, params.wind_speed, params.fetch);
                let theta = f64::atan2(ky, kx) - params.wind_direction;
                let s = spreading_exponent(w, omega_p, params.wind_speed, params.swell);
                let density = s_omega * tma(w, params.depth) * spreading(theta, s) * dw_dk / kk;
                let amplitude = (density * dk * dk).sqrt();
                // Two Gaussians by Box–Muller from the seed and the sample's index.
                let mut rng = params.seed.derive((y * n + x) as u64).rng();
                let (u1, u2) = (rng.next_f64().max(1.0e-12), rng.next_f64());
                let radius = (-2.0 * ln(u1)).sqrt();
                let (sn, cs) = sin_cos(std::f64::consts::TAU * u2);
                h0.push(
                    C(radius * cs, radius * sn).scale(amplitude * std::f64::consts::FRAC_1_SQRT_2),
                );
                omega.push(w);
            }
        }
        Self {
            params,
            k,
            h0,
            omega,
        }
    }

    /// The significant wave height, metres: four times the root of the spectrum's energy.
    pub fn significant_wave_height(&self) -> f64 {
        let energy: f64 = self.h0.iter().map(|c| c.0 * c.0 + c.1 * c.1).sum();
        4.0 * energy.sqrt()
    }

    /// The surface `time` seconds in.
    pub fn surface(&self, time: f64) -> OceanSurface {
        let n = self.params.size as usize;
        let count = n * n;
        // h(k, t) = h0(k) e^{iωt} + h0*(−k) e^{−iωt}.
        let index = |x: usize, y: usize| y * n + x;
        let mut h = vec![C::default(); count];
        for y in 0..n {
            for x in 0..n {
                let i = index(x, y);
                let j = index((n - x) % n, (n - y) % n);
                let phase = C::phasor(self.omega[i] * time);
                h[i] = self.h0[i]
                    .mul(phase)
                    .add(self.h0[j].conj().mul(phase.conj()));
            }
        }
        let field = |spectrum: &dyn Fn(usize, C) -> C| -> Field2<f32> {
            let mut grid: Vec<C> = (0..count).map(|i| spectrum(i, h[i])).collect();
            ifft2(&mut grid, n);
            Field2 {
                size: self.params.size,
                spacing: self.params.patch / n as f64,
                data: grid.iter().map(|c| c.0 as f32).collect(),
            }
        };
        let unit = |i: usize| {
            let (kx, ky) = self.k[i];
            let kk = (kx * kx + ky * ky).sqrt();
            if kk > 0.0 {
                (kx / kk, ky / kk, kx, ky)
            } else {
                (0.0, 0.0, 0.0, 0.0)
            }
        };
        let lambda = self.params.choppiness;
        let height = field(&|_, c| c);
        // Displacement −i k̂ h; slopes i k h; the displacement's derivatives k̂ k h (real).
        let dx = field(&|i, c| {
            let (ux, _, _, _) = unit(i);
            C(c.1 * ux, -c.0 * ux).scale(lambda)
        });
        let dy = field(&|i, c| {
            let (_, uy, _, _) = unit(i);
            C(c.1 * uy, -c.0 * uy).scale(lambda)
        });
        let slope_x = field(&|i, c| {
            let (_, _, kx, _) = unit(i);
            C(-c.1 * kx, c.0 * kx)
        });
        let slope_y = field(&|i, c| {
            let (_, _, _, ky) = unit(i);
            C(-c.1 * ky, c.0 * ky)
        });
        let dxx = field(&|i, c| {
            let (ux, _, kx, _) = unit(i);
            c.scale(ux * kx * lambda)
        });
        let dyy = field(&|i, c| {
            let (_, uy, _, ky) = unit(i);
            c.scale(uy * ky * lambda)
        });
        let dxy = field(&|i, c| {
            let (ux, _, _, ky) = unit(i);
            c.scale(ux * ky * lambda)
        });
        let jacobian = Field2 {
            size: self.params.size,
            spacing: height.spacing,
            data: (0..count)
                .map(|i| {
                    let (a, d, b) = (1.0 + dxx.data[i], 1.0 + dyy.data[i], dxy.data[i]);
                    a * d - b * b
                })
                .collect(),
        };
        OceanSurface {
            height,
            dx,
            dy,
            slope_x,
            slope_y,
            jacobian,
        }
    }
}

/// The numbers of a surface, for a report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanReport {
    /// The significant wave height from the spectrum, metres.
    pub significant_height: f64,
    /// The lowest and highest sample of the surface, metres.
    pub lowest: f32,
    /// See `lowest`.
    pub highest: f32,
    /// The largest horizontal displacement, metres.
    pub displacement: f32,
    /// The share of samples whose Jacobian is below zero (folded: foam).
    pub folded: f64,
    /// How long the transforms took.
    pub seconds: f64,
}

/// The surface at `time` and its numbers.
pub fn report(ocean: &Ocean, time: f64) -> (OceanSurface, OceanReport) {
    let start = Instant::now();
    let surface = ocean.surface(time);
    let seconds = start.elapsed().as_secs_f64();
    let (lowest, highest) = surface.height.min_max();
    let displacement = surface
        .dx
        .data
        .iter()
        .zip(&surface.dy.data)
        .map(|(x, y)| (x * x + y * y).sqrt())
        .fold(0.0, f32::max);
    let folded = surface.jacobian.data.iter().filter(|&&j| j < 0.0).count() as f64
        / surface.jacobian.len() as f64;
    (
        surface,
        OceanReport {
            significant_height: ocean.significant_wave_height(),
            lowest,
            highest,
            displacement,
            folded,
            seconds,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inverse_transform_turns_one_wave_vector_into_a_cosine() {
        let n = 16;
        let mut grid = vec![C::default(); n * n];
        // A at k = (1, 0) and its conjugate at (−1, 0): 2A cos(2πx/n).
        let a = C(0.5, 0.0);
        grid[1] = a;
        grid[n - 1] = a.conj();
        ifft2(&mut grid, n);
        for y in 0..n {
            for x in 0..n {
                let expected = (std::f64::consts::TAU * x as f64 / n as f64).cos();
                let got = grid[y * n + x];
                assert!(
                    (got.0 - expected).abs() < 1e-9 && got.1.abs() < 1e-9,
                    "({x},{y})"
                );
            }
        }
    }

    #[test]
    fn a_breeze_raises_metres_of_waves_the_same_every_time() {
        let params = OceanParams {
            size: 64,
            patch: 128.0,
            ..OceanParams::breeze(Seed::new(5))
        };
        let ocean = Ocean::new(params);
        let hs = ocean.significant_wave_height();
        assert!(hs > 0.5 && hs < 6.0, "significant height {hs}");
        let (surface, numbers) = report(&ocean, 3.0);
        // Zero mean, real, a height of the order of Hs, some displacement, little folding.
        let mean: f32 = surface.height.data.iter().sum::<f32>() / surface.height.len() as f32;
        assert!(mean.abs() < 0.05 * hs as f32, "mean {mean}");
        assert!(numbers.highest > 0.2 * hs as f32 && numbers.highest < 1.5 * hs as f32);
        assert!(numbers.displacement > 0.0 && numbers.folded < 0.2);
        // The slopes are the height's derivative, roughly (a central difference).
        let (n, dx) = (surface.height.size, surface.height.spacing as f32);
        let (x, y) = (10, 20);
        let finite = (surface.height.get(x + 1, y) - surface.height.get(x - 1, y)) / (2.0 * dx);
        assert!((surface.slope_x.get(x, y) - finite).abs() < 0.5 * finite.abs().max(0.02));
        assert_eq!(n, 64);
        // Deterministic; another seed is another sea; time moves it.
        assert_eq!(Ocean::new(params).surface(3.0), surface);
        let other = Ocean::new(OceanParams {
            seed: Seed::new(6),
            ..params
        });
        assert_ne!(other.surface(3.0).height, surface.height);
        assert_ne!(ocean.surface(4.0).height, surface.height);
        // Deep water and no swell still work.
        let deep = Ocean::new(OceanParams {
            depth: f64::INFINITY,
            swell: 0.0,
            ..params
        });
        assert!(deep.significant_wave_height() > hs * 0.5);
    }
}

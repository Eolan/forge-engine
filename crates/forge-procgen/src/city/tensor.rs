//! The street pattern as a tensor field (Chen, Esch, Wonka, Müller & Zhang 2008, "Interactive
//! procedural street modeling"): a symmetric, traceless 2×2 tensor per point,
//! `R · [[cos 2θ, sin 2θ], [sin 2θ, −cos 2θ]]`, whose major eigenvector (angle θ) and minor one
//! (θ + 90°) give the two families of streets. Basis fields add up: a grid per district at its
//! own angle, a radial pattern at squares, and the coast and rivers' tangents near the water,
//! each weighted by a Gaussian of the distance. The sum is baked on a grid and sampled
//! bilinearly; θ is `½·atan2(b, a)` with `forge_core::dmath` (D-016).

use forge_core::dmath;
use glam::DVec2;

use super::geom::{self, PointGrid};
use super::{DistrictSpec, Pattern, Site};
use crate::noise;

/// One element of the sum: `(a, b) = R·(cos 2θ, sin 2θ)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tensor {
    /// `R·cos 2θ`.
    pub a: f64,
    /// `R·sin 2θ`.
    pub b: f64,
}

impl Tensor {
    /// The tensor of a grid whose major streets run at angle `theta` (radians, from +x).
    pub fn grid(theta: f64) -> Self {
        let (s, c) = dmath::sin_cos(2.0 * theta);
        Self { a: c, b: s }
    }

    /// The tensor of a radial pattern around `centre` at `p`: rings as the major family,
    /// spokes as the minor one.
    pub fn radial(p: DVec2, centre: DVec2) -> Self {
        let d = p - centre;
        let len2 = d.length_squared();
        if len2 < 1e-6 {
            return Self::default();
        }
        // Rings: the major direction is perpendicular to d.
        Self {
            a: (d.y * d.y - d.x * d.x) / len2,
            b: -2.0 * d.x * d.y / len2,
        }
    }

    /// The same tensor turned by `phi` radians.
    pub fn rotated(self, phi: f64) -> Self {
        let (s, c) = dmath::sin_cos(2.0 * phi);
        Self {
            a: self.a * c - self.b * s,
            b: self.a * s + self.b * c,
        }
    }

    /// The major direction's angle, or `None` at a degenerate point.
    pub fn major_angle(self) -> Option<f64> {
        (self.a * self.a + self.b * self.b > 1e-8).then(|| 0.5 * dmath::atan2(self.b, self.a))
    }
}

impl std::ops::Add for Tensor {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self {
            a: self.a + o.a,
            b: self.b + o.b,
        }
    }
}

impl std::ops::Mul<f64> for Tensor {
    type Output = Self;
    fn mul(self, w: f64) -> Self {
        Self {
            a: self.a * w,
            b: self.b * w,
        }
    }
}

/// The baked field.
#[derive(Clone, Debug)]
pub struct TensorField {
    size: f64,
    cell: f64,
    side: usize,
    samples: Vec<Tensor>,
}

/// How strongly and how far the water bends the streets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundaryInfluence {
    /// The water's weight against a district's at the shore.
    pub strength: f64,
    /// The Gaussian's radius, metres.
    pub reach: f64,
}

impl TensorField {
    /// Bakes the sum of the districts' patterns and the water's tangents on a grid of `cell`
    /// metres.
    pub fn bake(
        site: &Site,
        districts: &[DistrictSpec],
        boundary: BoundaryInfluence,
        seed: u64,
        cell: f64,
    ) -> Self {
        let side = (site.size / cell).ceil() as usize + 1;
        // The water's edges, sampled every 20 m with their tangents.
        let edges = water_edges(site);
        let mut edge_grid = PointGrid::new(site.size, boundary.reach.max(cell));
        for (i, &(p, _)) in edges.iter().enumerate() {
            edge_grid.insert(p, i as u32);
        }
        let organic_seed = seed ^ 0x5EED_C17E;
        let mut samples = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                let p = DVec2::new(x as f64 * cell, y as f64 * cell);
                let mut t = Tensor::default();
                for d in districts {
                    let dist = p.distance(d.anchor) / d.radius;
                    let w = dmath::exp(-dist * dist);
                    if w < 1e-4 {
                        continue;
                    }
                    let mut e = match d.pattern {
                        Pattern::Radial => Tensor::radial(p, d.anchor),
                        Pattern::Grid | Pattern::Organic(_) => Tensor::grid(d.grid_angle),
                    };
                    if let Pattern::Organic(amplitude) = d.pattern {
                        // Lanes that wander: the grid turned by a slow noise.
                        let n = noise::fbm(organic_seed, p.x / 180.0, p.y / 180.0, 3, 2.0, 0.5);
                        e = e.rotated(n * amplitude);
                    }
                    t = t + e * w;
                }
                if let Some((q, i)) = edge_grid.nearest(p, 3.0 * boundary.reach) {
                    let dist = p.distance(q) / boundary.reach;
                    let w = boundary.strength * dmath::exp(-dist * dist);
                    let tangent = edges[i as usize].1;
                    t = t + Tensor::grid(dmath::atan2(tangent.y, tangent.x)) * w;
                }
                samples.push(t);
            }
        }
        Self {
            size: site.size,
            cell,
            side,
            samples,
        }
    }

    /// The field at `p`, bilinear between the baked samples.
    pub fn at(&self, p: DVec2) -> Tensor {
        let fx = (p.x / self.cell).clamp(0.0, (self.side - 1) as f64);
        let fy = (p.y / self.cell).clamp(0.0, (self.side - 1) as f64);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.side - 1), (y0 + 1).min(self.side - 1));
        let (tx, ty) = (fx - x0 as f64, fy - y0 as f64);
        let s = |x: usize, y: usize| self.samples[y * self.side + x];
        let top = s(x0, y0) * (1.0 - tx) + s(x1, y0) * tx;
        let bottom = s(x0, y1) * (1.0 - tx) + s(x1, y1) * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// The unit direction of the major (`minor = false`) or minor family at `p`, or `None` at a
    /// degenerate point.
    pub fn direction(&self, p: DVec2, minor: bool) -> Option<DVec2> {
        let theta = self.at(p).major_angle()?;
        let theta = if minor {
            theta + std::f64::consts::FRAC_PI_2
        } else {
            theta
        };
        let (s, c) = dmath::sin_cos(theta);
        Some(DVec2::new(c, s))
    }

    /// The frame's side, metres.
    pub fn size(&self) -> f64 {
        self.size
    }
}

/// Points on the edges between land and water every ~20 m, with the edge's tangent: the coast
/// from the water mask, each river from its centreline.
fn water_edges(site: &Site) -> Vec<(DVec2, DVec2)> {
    let mut edges = Vec::new();
    for river in &site.rivers {
        let line = geom::resample(&river.centreline, 20.0);
        let length = geom::polyline_length(&line);
        let mut s = 0.0;
        while s <= length {
            edges.push((geom::point_at(&line, s), geom::tangent_at(&line, s)));
            s += 20.0;
        }
    }
    // The sea's edge: a land cell next to a sea cell, its tangent across the mask's gradient.
    let water = &site.water;
    let n = water.size;
    let stride = ((20.0 / water.spacing).round() as u32).max(1);
    for y in (1..n - 1).step_by(stride as usize) {
        for x in (1..n - 1).step_by(stride as usize) {
            if water.get(x, y) != super::LAND {
                continue;
            }
            let sea = |dx: i32, dy: i32| {
                let v = water.get((x as i32 + dx) as u32, (y as i32 + dy) as u32);
                f64::from(u8::from(v == super::SEA))
            };
            let gx = sea(1, 0) - sea(-1, 0);
            let gy = sea(0, 1) - sea(0, -1);
            if gx == 0.0 && gy == 0.0 {
                continue;
            }
            let p = DVec2::new(x as f64 * water.spacing, y as f64 * water.spacing);
            edges.push((p, DVec2::new(-gy, gx).normalize()));
        }
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_s_major_angle_is_its_own() {
        for &theta in &[0.0, 0.3, 1.2, -0.7] {
            let back = Tensor::grid(theta).major_angle().expect("not degenerate");
            // θ and θ ± π are the same street direction.
            let diff = (back - theta).rem_euclid(std::f64::consts::PI);
            assert!(diff < 1e-12 || (std::f64::consts::PI - diff) < 1e-12);
        }
    }

    #[test]
    fn a_radial_field_makes_rings() {
        let t = Tensor::radial(DVec2::new(10.0, 0.0), DVec2::ZERO);
        let theta = t.major_angle().expect("not degenerate");
        // East of the centre the ring runs north–south.
        assert!((dmath::cos(theta)).abs() < 1e-12);
    }

    #[test]
    fn rotation_turns_the_streets() {
        let t = Tensor::grid(0.2).rotated(0.5);
        let theta = t.major_angle().expect("not degenerate");
        assert!((theta - 0.7).abs() < 1e-12);
    }
}

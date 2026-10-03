//! Shallow water on a grid (issue #144, Phase 3's step 8): D-009's middle tier, the
//! authoritative column model. A column of water stands on each cell of a bed; the water's
//! velocities live on the faces between cells (a staggered grid). Each step (after Matthias
//! Müller-Fischer's height-field water, "Fast Water Simulation for Games Using Height Fields",
//! GDC 2008): the velocities are carried along by themselves (semi-Lagrangian), the water
//! flows through each face at its velocity times the depth it leaves (upwind), each cell's
//! outflow scaled down so that it never gives more water than it holds, and the slope of the
//! surface across each face speeds its velocity up (the shallow-water equations' g ∂η/∂x). A
//! face into a cell whose bed stands over the water's surface carries nothing. The volume is
//! kept to the rounding, no column goes below zero, and a dam break's front runs as the water
//! carries its speed. The grid's edges are walls. What floats pushes the water aside in turn
//! (#151, [`Pool::displace`]): its volume under the water, as a thickness over its footprint,
//! adds to the surface the slopes see.
//!
//! Sums, products, quotients, floors and roundings of `f32` and `f64` in a fixed order and
//! nothing else, so the same bytes on every platform; the state is the depths, the faces'
//! velocities and what floats, saved and restored with the rest.

use glam::Vec3;

use crate::buoyancy::Water;

/// Water thinner than this, metres, counts as dry for its surface and its speed.
pub const DRY: f32 = 0.002;
/// The most a face's velocity carries a step, in cells (stability).
const MOST_CELLS_A_STEP: f32 = 0.5;

/// A grid of water columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Pool {
    /// Cells along x and along z.
    pub size: [usize; 2],
    /// Metres between cells' centres.
    pub spacing: f32,
    /// World x and z of the first cell's centre.
    pub origin: [f64; 2],
    /// The bed's height under each cell, metres (row-major, rows along +z).
    pub bed: Vec<f32>,
    /// The water's depth on each cell, metres.
    pub depth: Vec<f32>,
    /// The thickness of what floats in each cell's water, metres (two-way coupling, #151): set
    /// each step from the bodies, it adds to the surface the water's slopes see, so the water
    /// flows out from under a body and rises round it, and a body afloat sees the level it would
    /// have without it. The depths keep the volume; this is not water.
    pub displaced: Vec<f32>,
    /// The velocity along x on the faces between cells, m/s: `(nx + 1) × nz`, face `x` of a
    /// row on cell `x`'s −x side (the edges' faces are walls, always 0).
    u: Vec<f32>,
    /// The velocity along z on the faces between rows: `nx × (nz + 1)`.
    w: Vec<f32>,
    /// How much of its velocity the water loses a second to the bed's friction.
    pub friction: f32,
    /// m/s², downwards.
    pub gravity: f32,
}

impl Pool {
    /// A dry pool of `size` cells `spacing` apart, its first cell's centre at `origin`, on a
    /// flat bed at 0.
    pub fn new(size: [usize; 2], spacing: f32, origin: [f64; 2]) -> Self {
        let [nx, nz] = size;
        Self {
            size,
            spacing,
            origin,
            bed: vec![0.0; nx * nz],
            depth: vec![0.0; nx * nz],
            displaced: vec![0.0; nx * nz],
            u: vec![0.0; (nx + 1) * nz],
            w: vec![0.0; nx * (nz + 1)],
            friction: 0.3,
            gravity: 9.81,
        }
    }

    /// The cell `(x, z)`'s index.
    pub fn index(&self, x: usize, z: usize) -> usize {
        z * self.size[0] + x
    }

    /// The water's surface on cell `i` (its bed where it is dry), with what floats in it.
    pub fn surface(&self, i: usize) -> f32 {
        self.bed[i] + self.depth[i] + self.displaced[i]
    }

    /// The water held, m³.
    pub fn volume(&self) -> f64 {
        let area = f64::from(self.spacing) * f64::from(self.spacing);
        self.depth.iter().map(|&d| f64::from(d)).sum::<f64>() * area
    }

    /// What floats in the water (#151), from `bodies`: each its world (x, z), its volume under
    /// the water, m³, and the radius of its footprint, metres. Each volume is spread evenly, as a
    /// thickness, over the wet cells whose centres lie in its footprint (the nearest wet cell
    /// when none does), in the bodies' order.
    pub fn displace(&mut self, bodies: &[([f64; 2], f32, f32)]) {
        self.displaced.fill(0.0);
        let [nx, nz] = self.size;
        let l = f64::from(self.spacing);
        let area = self.spacing * self.spacing;
        let mut cells = Vec::new();
        for &([x, z], volume, radius) in bodies {
            if volume <= 0.0 {
                continue;
            }
            // In cells, from the first cell's centre.
            let (gx, gz) = ((x - self.origin[0]) / l, (z - self.origin[1]) / l);
            let r = f64::from(radius) / l;
            let span = |g: f64, n: usize| {
                let low = (g - r).ceil().max(0.0);
                let high = (g + r).floor().min((n - 1) as f64);
                (low as usize, high as usize, low <= high)
            };
            let ((x0, x1, xs), (z0, z1, zs)) = (span(gx, nx), span(gz, nz));
            cells.clear();
            if xs && zs {
                for cz in z0..=z1 {
                    for cx in x0..=x1 {
                        let (dx, dz) = (cx as f64 - gx, cz as f64 - gz);
                        let i = self.index(cx, cz);
                        if dx * dx + dz * dz <= r * r && self.depth[i] >= DRY {
                            cells.push(i);
                        }
                    }
                }
            }
            if cells.is_empty() {
                let (cx, cz) = (gx.round(), gz.round());
                if cx < 0.0 || cz < 0.0 || cx > (nx - 1) as f64 || cz > (nz - 1) as f64 {
                    continue;
                }
                let i = self.index(cx as usize, cz as usize);
                if self.depth[i] < DRY {
                    continue;
                }
                cells.push(i);
            }
            let each = volume / (cells.len() as f32 * area);
            for &i in &cells {
                self.displaced[i] += each;
            }
        }
    }

    /// Advances the water by `dt` seconds, in two halves.
    pub fn step(&mut self, dt: f32) {
        for _ in 0..2 {
            self.advect(0.5 * dt);
            self.flow(0.5 * dt);
            self.accelerate(0.5 * dt);
        }
    }

    /// The velocities carried along by themselves for `dt`: each face's taken from where its
    /// water was `dt` ago (in each grid's own units: u's face `x` of row `z` at `(x, z)`, w's
    /// face `x` of row-face `z` at `(x, z)`).
    fn advect(&mut self, dt: f32) {
        let [nx, nz] = self.size;
        let cells = dt / self.spacing;
        let mut u = self.u.clone();
        for z in 0..nz {
            for x in 1..nx {
                // The w faces round this u face: columns x − 1 and x, row-faces z and z + 1.
                let along = arriving(
                    self.u[z * (nx + 1) + x - 1],
                    self.u[z * (nx + 1) + x],
                    self.u[z * (nx + 1) + x + 1],
                );
                let across = 0.25
                    * ((self.w[z * nx + x - 1] + self.w[z * nx + x])
                        + (self.w[(z + 1) * nx + x - 1] + self.w[(z + 1) * nx + x]));
                u[z * (nx + 1) + x] = sample(
                    &self.u,
                    nx + 1,
                    nz,
                    x as f32 - along * cells,
                    z as f32 - across * cells,
                );
            }
        }
        let mut w = self.w.clone();
        for z in 1..nz {
            for x in 0..nx {
                // The u faces round this w face: faces x and x + 1, rows z − 1 and z.
                let along = arriving(
                    self.w[(z - 1) * nx + x],
                    self.w[z * nx + x],
                    self.w[(z + 1) * nx + x],
                );
                let across = 0.25
                    * ((self.u[(z - 1) * (nx + 1) + x] + self.u[(z - 1) * (nx + 1) + x + 1])
                        + (self.u[z * (nx + 1) + x] + self.u[z * (nx + 1) + x + 1]));
                w[z * nx + x] = sample(
                    &self.w,
                    nx,
                    nz + 1,
                    x as f32 - across * cells,
                    z as f32 - along * cells,
                );
            }
        }
        self.u = u;
        self.w = w;
    }

    /// The water through the faces for `dt`: each face's velocity times the depth on its
    /// upwind side, each cell's outflow scaled to what it holds.
    fn flow(&mut self, dt: f32) {
        let [nx, nz] = self.size;
        let area = self.spacing * self.spacing;
        let l = self.spacing;
        // The volume through each face this step, positive along +x and +z.
        let mut fu = vec![0.0_f32; (nx + 1) * nz];
        let mut fw = vec![0.0_f32; nx * (nz + 1)];
        for z in 0..nz {
            for x in 1..nx {
                let v = self.u[z * (nx + 1) + x];
                let up = if v > 0.0 { z * nx + x - 1 } else { z * nx + x };
                fu[z * (nx + 1) + x] = v * self.depth[up] * l * dt;
            }
        }
        for z in 1..nz {
            for x in 0..nx {
                let v = self.w[z * nx + x];
                let up = if v > 0.0 {
                    (z - 1) * nx + x
                } else {
                    z * nx + x
                };
                fw[z * nx + x] = v * self.depth[up] * l * dt;
            }
        }
        // Each cell's outflow, and the share of it it can give.
        let mut give = vec![1.0_f32; nx * nz];
        for z in 0..nz {
            for x in 0..nx {
                let i = z * nx + x;
                let out = (fu[z * (nx + 1) + x + 1].max(0.0) + (-fu[z * (nx + 1) + x]).max(0.0))
                    + (fw[(z + 1) * nx + x].max(0.0) + (-fw[z * nx + x]).max(0.0));
                let holds = self.depth[i] * area;
                if out > holds && out > 0.0 {
                    give[i] = holds / out;
                }
            }
        }
        for z in 0..nz {
            for x in 1..nx {
                let f = &mut fu[z * (nx + 1) + x];
                *f *= if *f > 0.0 {
                    give[z * nx + x - 1]
                } else {
                    give[z * nx + x]
                };
            }
        }
        for z in 1..nz {
            for x in 0..nx {
                let f = &mut fw[z * nx + x];
                *f *= if *f > 0.0 {
                    give[(z - 1) * nx + x]
                } else {
                    give[z * nx + x]
                };
            }
        }
        for z in 0..nz {
            for x in 0..nx {
                let i = z * nx + x;
                let net = (fu[z * (nx + 1) + x] - fu[z * (nx + 1) + x + 1])
                    + (fw[z * nx + x] - fw[(z + 1) * nx + x]);
                let was_dry = self.depth[i] < DRY;
                self.depth[i] = (self.depth[i] + net / area).max(0.0);
                // A dry cell the water runs into takes up its speed: the face on its far side
                // carries the water on as fast as it came (the momentum comes with the water,
                // which tracing the faces back smears out over steps, and a front crawled).
                if was_dry && self.depth[i] >= DRY {
                    let (left, right) = (z * (nx + 1) + x, z * (nx + 1) + x + 1);
                    if fu[left] > 0.0 && x + 1 < nx {
                        self.u[right] = self.u[right].max(self.u[left]);
                    }
                    if fu[right] < 0.0 && x > 0 {
                        self.u[left] = self.u[left].min(self.u[right]);
                    }
                    let (near, far) = (z * nx + x, (z + 1) * nx + x);
                    if fw[near] > 0.0 && z + 1 < nz {
                        self.w[far] = self.w[far].max(self.w[near]);
                    }
                    if fw[far] < 0.0 && z > 0 {
                        self.w[near] = self.w[near].min(self.w[far]);
                    }
                }
            }
        }
    }

    /// The faces' velocities sped up by the surface's slope across them for `dt`, slowed by
    /// the bed, capped for stability; none between dry cells or into a bed over the water.
    fn accelerate(&mut self, dt: f32) {
        let [nx, nz] = self.size;
        let l = self.spacing;
        let keep = (1.0 - self.friction * dt).max(0.0);
        let most = MOST_CELLS_A_STEP * l / dt;
        let face = |p: &Self, v: f32, a: usize, b: usize| -> f32 {
            let (ha, hb) = (p.surface(a), p.surface(b));
            let (da, db) = (p.depth[a], p.depth[b]);
            if da < DRY && db < DRY {
                return 0.0;
            }
            let v = (v - dt * p.gravity * (hb - ha) / l) * keep;
            // Out of a dry cell, or into one whose bed stands over the water: none.
            let blocked = if v > 0.0 {
                da < DRY || p.bed[b] > ha
            } else {
                db < DRY || p.bed[a] > hb
            };
            if blocked { 0.0 } else { v.clamp(-most, most) }
        };
        for z in 0..nz {
            for x in 1..nx {
                let k = z * (nx + 1) + x;
                self.u[k] = face(self, self.u[k], z * nx + x - 1, z * nx + x);
            }
        }
        for z in 1..nz {
            for x in 0..nx {
                let k = z * nx + x;
                self.w[k] = face(self, self.w[k], (z - 1) * nx + x, z * nx + x);
            }
        }
    }

    /// The velocities on the faces: along x, `(nx + 1) × nz` (face `x` of a row on cell `x`'s −x
    /// side), and along z, `nx × (nz + 1)`; m/s. For a finer layer that shadows the pool (#162).
    pub fn faces(&self) -> (&[f32], &[f32]) {
        (&self.u, &self.w)
    }

    /// The water's mean velocity on cell `(x, z)` (world x, z; m/s), zero where it is dry.
    pub fn velocity_at(&self, x: usize, z: usize) -> [f32; 2] {
        let nx = self.size[0];
        if self.depth[z * nx + x] < DRY {
            return [0.0, 0.0];
        }
        [
            0.5 * (self.u[z * (nx + 1) + x] + self.u[z * (nx + 1) + x + 1]),
            0.5 * (self.w[z * nx + x] + self.w[(z + 1) * nx + x]),
        ]
    }

    /// The cell under world (x, z) and the share of the way to the next along each axis, for
    /// a bilinear lookup; `None` off the grid.
    fn cell(&self, x: f64, z: f64) -> Option<([usize; 2], [f32; 2])> {
        let gx = (x - self.origin[0]) / f64::from(self.spacing);
        let gz = (z - self.origin[1]) / f64::from(self.spacing);
        let [nx, nz] = self.size;
        if gx < 0.0 || gz < 0.0 || gx > (nx - 1) as f64 || gz > (nz - 1) as f64 {
            return None;
        }
        let (cx, cz) = ((gx as usize).min(nx - 2), (gz as usize).min(nz - 2));
        Some(([cx, cz], [(gx - cx as f64) as f32, (gz - cz as f64) as f32]))
    }

    /// The depths, the velocities, the bed and what floats as bytes, for a saved state.
    pub fn save(&self, out: &mut Vec<u8>) {
        for v in self
            .depth
            .iter()
            .chain(&self.u)
            .chain(&self.w)
            .chain(&self.bed)
            .chain(&self.displaced)
        {
            out.extend_from_slice(&v.to_bits().to_le_bytes());
        }
    }

    /// Back to a state [`Pool::save`] wrote; the bytes it read.
    pub fn restore(&mut self, bytes: &[u8]) -> usize {
        let mut words = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&w| f32::from_bits(u32::from_le_bytes(w)));
        let mut read = 0;
        for v in self
            .depth
            .iter_mut()
            .chain(&mut self.u)
            .chain(&mut self.w)
            .chain(&mut self.bed)
            .chain(&mut self.displaced)
        {
            *v = words.next().unwrap_or(0.0);
            read += 4;
        }
        read
    }

    /// A hash of the water to the bit.
    pub fn digest(&self) -> u64 {
        let mut bytes = Vec::new();
        self.save(&mut bytes);
        xxhash_rust::xxh3::xxh3_64(&bytes)
    }
}

/// The velocity a face's water is traced back along: its own, or its upstream neighbour's
/// where that one carries water towards it faster. A still face ahead of a front traced back
/// along its own nothing and never took up the water's speed: the front crawled at a cell
/// every few steps.
fn arriving(before: f32, own: f32, after: f32) -> f32 {
    if before > 0.0 && before > own {
        before
    } else if after < 0.0 && after < own {
        after
    } else {
        own
    }
}

/// A grid of `nx × nz` values sampled bilinearly at `(x, z)` in its own units, clamped to it.
fn sample(values: &[f32], nx: usize, nz: usize, x: f32, z: f32) -> f32 {
    let x = x.clamp(0.0, (nx - 1) as f32);
    let z = z.clamp(0.0, (nz - 1) as f32);
    let (x0, z0) = ((x as usize).min(nx - 2), (z as usize).min(nz - 2));
    let (tx, tz) = (x - x0 as f32, z - z0 as f32);
    let at = |dx: usize, dz: usize| values[(z0 + dz) * nx + x0 + dx];
    let a = at(0, 0) + (at(1, 0) - at(0, 0)) * tx;
    let b = at(0, 1) + (at(1, 1) - at(0, 1)) * tx;
    a + (b - a) * tz
}

impl Pool {
    /// The bilinear weights of the four cells round world (x, z) that hold water, with their
    /// indices; empty off the grid or where all four are dry.
    fn wet_round(&self, x: f64, z: f64) -> Vec<(usize, f32)> {
        let Some(([cx, cz], [tx, tz])) = self.cell(x, z) else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(4);
        for (dx, dz, w) in [
            (0, 0, (1.0 - tx) * (1.0 - tz)),
            (1, 0, tx * (1.0 - tz)),
            (0, 1, (1.0 - tx) * tz),
            (1, 1, tx * tz),
        ] {
            let i = self.index(cx + dx, cz + dz);
            if self.depth[i] >= DRY && w > 0.0 {
                out.push((i, w));
            }
        }
        out
    }
}

impl Water for Pool {
    /// The surface of the wet cells round the point, weighted bilinearly: a dry cell (a bank,
    /// the dam) adds nothing, so what lies against it is not pushed by water that is not
    /// there. Where none is wet, or off the grid, far below.
    fn height(&self, x: f64, z: f64) -> f64 {
        let wet = self.wet_round(x, z);
        let total: f32 = wet.iter().map(|&(_, w)| w).sum();
        if total <= 0.0 {
            return -1.0e6;
        }
        f64::from(wet.iter().map(|&(i, w)| w * self.surface(i)).sum::<f32>() / total)
    }

    fn current(&self, x: f64, z: f64) -> Vec3 {
        let wet = self.wet_round(x, z);
        let total: f32 = wet.iter().map(|&(_, w)| w).sum();
        if total <= 0.0 {
            return Vec3::ZERO;
        }
        let nx = self.size[0];
        let v: glam::Vec2 = wet
            .iter()
            .map(|&(i, w)| w * glam::Vec2::from(self.velocity_at(i % nx, i / nx)))
            .sum::<glam::Vec2>()
            / total;
        Vec3::new(v.x, 0.0, v.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A channel 30 m long and 2 m wide at 0.25 m, a 1 m column of water over its first third
    /// behind a dam taken away at the start.
    fn dam_break() -> Pool {
        let mut pool = Pool::new([120, 8], 0.25, [0.0, 0.0]);
        pool.friction = 0.0;
        for z in 0..8 {
            for x in 0..40 {
                let i = pool.index(x, z);
                pool.depth[i] = 1.0;
            }
        }
        pool
    }

    #[test]
    fn the_water_is_kept_and_never_below_zero() {
        let mut pool = dam_break();
        let start = pool.volume();
        for _ in 0..600 {
            pool.step(1.0 / 60.0);
        }
        assert!(
            (pool.volume() - start).abs() < 1e-3 * start,
            "{} of {start}",
            pool.volume()
        );
        assert!(pool.depth.iter().all(|&d| d >= 0.0));
    }

    #[test]
    fn a_dam_break_follows_ritter_and_its_front_runs() {
        // Ritter's solution over a dry bed, for 1 m of water: at the dam's place the depth
        // stays 4⁄9 of it and the water runs at ⅔ √(g h₀), 2.09 m/s; its tip runs at
        // 2 √(g h₀), 6.3 m/s, which a first-order scheme lags where the water thins to nothing.
        let mut pool = dam_break();
        for _ in 0..60 {
            pool.step(1.0 / 60.0);
        }
        let at_dam = pool.depth[pool.index(40, 4)];
        let [u, w] = pool.velocity_at(40, 4);
        assert!(
            (at_dam / (4.0 / 9.0) - 1.0).abs() < 0.05,
            "{at_dam} m deep at the dam"
        );
        assert!(
            (u / 2.09 - 1.0).abs() < 0.1 && w.abs() < 0.01,
            "{u} {w} m/s at the dam"
        );
        // The front after a second: the farthest cell wetter than a centimetre, 3 m on at
        // least (3.25 m at the time of writing).
        let front = (0..120)
            .rev()
            .find(|&x| pool.depth[pool.index(x, 4)] > 0.01)
            .unwrap() as f32
            * 0.25
            - 10.0;
        assert!((3.0..7.0).contains(&front), "the front ran {front} m");
    }

    #[test]
    fn a_saved_pool_runs_on_to_the_same_bits() {
        let mut pool = dam_break();
        for _ in 0..30 {
            pool.step(1.0 / 60.0);
        }
        let mut saved = Vec::new();
        pool.save(&mut saved);
        for _ in 0..60 {
            pool.step(1.0 / 60.0);
        }
        let first = pool.digest();
        let read = pool.restore(&saved);
        assert_eq!(read, saved.len());
        for _ in 0..60 {
            pool.step(1.0 / 60.0);
        }
        assert_eq!(pool.digest(), first);
    }

    #[test]
    fn still_water_floats_a_box_at_its_level() {
        let mut pool = Pool::new([20, 20], 0.5, [-5.0, -5.0]);
        pool.depth.fill(1.5);
        for _ in 0..60 {
            pool.step(1.0 / 60.0);
        }
        assert!((pool.height(0.3, -0.7) - 1.5).abs() < 1e-4);
        assert!(pool.current(0.3, -0.7).length() < 1e-4);
        assert!(pool.height(50.0, 0.0) < -1000.0);
    }

    #[test]
    fn what_floats_pushes_the_water_aside() {
        // Still water 1 m deep in a 10 m square, and a body of 0.4 m³ set into it at once over
        // a footprint 0.6 m round: the water flows out from under it as a ring, keeps its
        // volume, and settles level round it, the whole pool a little higher.
        let mut pool = Pool::new([40, 40], 0.25, [-4.875, -4.875]);
        pool.depth.fill(1.0);
        let before = pool.volume();
        let body = [([0.0, 0.0], 0.4, 0.6)];
        let ring = pool.index(20 + 6, 20);
        let mut highest = 0.0_f32;
        for _ in 0..60 {
            pool.displace(&body);
            pool.step(1.0 / 60.0);
            highest = highest.max(pool.depth[ring]);
        }
        assert!(highest > 1.005, "the ring 1.5 m out rose to {highest}");
        // The ring sloshes round the box, losing half the friction's rate (its energy is half in
        // its speed): 40 s to settle under a millimetre.
        for _ in 0..2400 {
            pool.displace(&body);
            pool.step(1.0 / 60.0);
        }
        assert!(
            (pool.volume() - before).abs() < 1e-3,
            "{}",
            pool.volume() - before
        );
        // Under the body the water is thinner by its thickness; its surface and the far water's
        // stand level, 0.4 m³ over 100 m² (4 mm) over the first.
        let under = pool.index(20, 20);
        let far = pool.index(2, 2);
        assert!(pool.depth[under] < 0.7, "{}", pool.depth[under]);
        assert!(
            (pool.surface(under) - pool.surface(far)).abs() < 2e-3,
            "{} {} {} {}",
            pool.surface(under),
            pool.surface(far),
            pool.depth[under],
            pool.displaced[under]
        );
        assert!(
            (pool.surface(far) - 1.004).abs() < 1e-3,
            "{}",
            pool.surface(far)
        );
    }
}

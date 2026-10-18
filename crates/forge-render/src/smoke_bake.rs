//! The particles' smoke flipbook (D-064's point 6, #271), baked by our own solver: a puff of
//! smoke simulated on a small grid with Jos Stam's "Stable Fluids" (SIGGRAPH 1999; US 6,266,071
//! B1 expired in 2018), then each of its frames ray-marched towards the camera lit from six
//! directions, as Unity's six-way smoke lighting stores them (`docs/research/particles.md` §3).
//!
//! - **The simulation:** velocity, density and heat at the cells' middles of a [`GRID`]³ box.
//!   For the first part of its life the puff's foot is fed smoke and heat. Each step adds
//!   buoyancy (heat rises, smoke weighs), vorticity confinement (Fedkiw, Stam and Jensen 2001,
//!   which keeps the curls numerical diffusion would smooth away), advects the velocity
//!   semi-Lagrangian, projects it divergence-free by Jacobi iterations on the pressure, then
//!   advects the smoke and the heat, which cools.
//! - **The frames:** [`FRAMES`] of them over the puff's life. Each pixel marches its column
//!   from the camera's side to the far side, single scattering, isotropic. The light reaching a
//!   cell from each of the six axis directions is the transmittance through the smoke between it
//!   and that side of the box: a running sum along the grid's lines, no ray per cell.
//! - **The atlases:** two RGBA8 images of [`COLUMNS`] × [`COLUMNS`] cells of [`TEXELS`]² each,
//!   frame `k` in cell `k`. The positive one holds the light from the right, from above and
//!   from behind (the far side) and the alpha; the negative one from the left, from below and
//!   from the camera's side. Lights are stored as their square roots, for precision in the
//!   darks.
//!
//! It runs in a second or so and is kept in the derived cache with the other generated
//! textures ([`crate::textures::keep_in`]).

use crate::textures::TextureData;

/// Cells a side of the simulation's box.
pub const GRID: usize = 64;
/// Frames of the flipbook, over a puff's life.
pub const FRAMES: usize = 32;
/// Cells a side of an atlas, frames in them row by row.
pub const COLUMNS: usize = 6;
/// Texels a side of a frame.
pub const TEXELS: usize = 96;
/// Simulation steps a frame.
const STEPS_PER_FRAME: usize = 3;
/// The time step, in the box's units of time.
const DT: f32 = 0.6;
/// Jacobi iterations of the pressure each step.
const PRESSURE_ITERATIONS: usize = 24;
/// How strongly heat lifts and smoke weighs.
const BUOYANCY: f32 = 0.012;
const WEIGHT: f32 = 0.004;
/// Vorticity confinement's strength.
const CONFINEMENT: f32 = 1.0;
/// The share of the heat kept each step, and of the smoke.
const COOLING: f32 = 0.97;
/// How much a unit of heat expands its cell each step: the divergence the pressure aims for
/// (Feldman, O'Brien and Arikan, "Animating Suspended Particle Explosions", SIGGRAPH 2003),
/// which grows the puff from its hot core as it cools.
const EXPANSION: f32 = 0.065;
const FADING: f32 = 0.995;
/// The steps the foot is fed for.
const FEED_STEPS: usize = 6;
/// The smoke's extinction per unit of density, per cell crossed.
const EXTINCTION: f32 = 0.6;

/// A field of numbers, one a cell, `x + GRID (y + GRID z)`.
type Field = Vec<f32>;

fn index(x: usize, y: usize, z: usize) -> usize {
    x + GRID * (y + GRID * z)
}

/// The field's value at a point in cells (cell middles at whole numbers), trilinear, clamped
/// to the box.
fn sample(f: &Field, p: [f32; 3]) -> f32 {
    let max = (GRID - 1) as f32;
    let c = p.map(|v| v.clamp(0.0, max));
    let i = c.map(|v| (v.floor() as usize).min(GRID - 2));
    let t = [c[0] - i[0] as f32, c[1] - i[1] as f32, c[2] - i[2] as f32];
    let at = |dx: usize, dy: usize, dz: usize| f[index(i[0] + dx, i[1] + dy, i[2] + dz)];
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(at(0, 0, 0), at(1, 0, 0), t[0]);
    let x10 = lerp(at(0, 1, 0), at(1, 1, 0), t[0]);
    let x01 = lerp(at(0, 0, 1), at(1, 0, 1), t[0]);
    let x11 = lerp(at(0, 1, 1), at(1, 1, 1), t[0]);
    lerp(lerp(x00, x10, t[1]), lerp(x01, x11, t[1]), t[2])
}

/// A number in 0..1 from three integers (Jarzynski and Olano's `pcg3d`, as the shaders').
fn hash(x: u32, y: u32, z: u32) -> f32 {
    let mut v = [
        x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223),
        y.wrapping_mul(1_664_525).wrapping_add(1_013_904_223),
        z.wrapping_mul(1_664_525).wrapping_add(1_013_904_223),
    ];
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v = v.map(|a| a ^ (a >> 16));
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    (v[0] >> 8) as f32 / 16_777_216.0
}

/// The puff's state.
struct Smoke {
    velocity: [Field; 3],
    density: Field,
    heat: Field,
    step: usize,
}

impl Smoke {
    fn new() -> Self {
        let zero = vec![0.0; GRID * GRID * GRID];
        Self {
            velocity: [zero.clone(), zero.clone(), zero.clone()],
            density: zero.clone(),
            heat: zero,
            step: 0,
        }
    }

    /// Feeds the puff: a burst in a ball a little under the box's middle, pushed outwards, its
    /// edge broken by noise that changes each step, so the smoke billows out in lumps. The
    /// particle carries the rise; the sprite's puff stays in its box.
    fn feed(&mut self) {
        let g = GRID as f32;
        let middle = [0.5 * g, 0.42 * g, 0.5 * g];
        // A cauliflower: a middle ball and six round it, each its own size, density and swirl
        // (a spin about a random axis), so the light finds lumps in the smoke to shade.
        let balls: Vec<([f32; 3], f32, f32, [f32; 3])> = (0..7_u32)
            .map(|k| {
                let r = |c: u32| hash(k, c, 3);
                let offset = if k == 0 {
                    [0.0; 3]
                } else {
                    let a = std::f32::consts::TAU * r(0);
                    let e = (r(1) - 0.5) * 1.2;
                    [a.cos() * e.cos(), e.sin(), a.sin() * e.cos()].map(|v| v * 0.09 * g)
                };
                let axis = [r(4) - 0.5, r(5) - 0.5, r(6) - 0.5];
                (
                    [
                        middle[0] + offset[0],
                        middle[1] + offset[1],
                        middle[2] + offset[2],
                    ],
                    (0.06 + 0.05 * r(2)) * g,
                    0.15 + 0.2 * r(3),
                    axis,
                )
            })
            .collect();
        for z in 0..GRID {
            for y in 0..GRID {
                for x in 0..GRID {
                    let p = [x as f32, y as f32, z as f32];
                    for (centre, radius, density, axis) in &balls {
                        let d = [p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]];
                        let r = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                        if r >= *radius {
                            continue;
                        }
                        let i = index(x, y, z);
                        let edge = 1.0 - r / radius;
                        self.density[i] += density * edge;
                        self.heat[i] += 0.5 * edge;
                        // The ball's spin: axis × d, a swirl the confinement keeps.
                        let spin = [
                            axis[1] * d[2] - axis[2] * d[1],
                            axis[2] * d[0] - axis[0] * d[2],
                            axis[0] * d[1] - axis[1] * d[0],
                        ];
                        for (field, s) in self.velocity.iter_mut().zip(spin) {
                            field[x + GRID * (y + GRID * z)] += 0.6 * edge * s / radius;
                        }
                    }
                }
            }
        }
    }

    /// One step of the simulation.
    fn advance(&mut self) {
        if self.step < FEED_STEPS {
            self.feed();
        }
        // Buoyancy, and vorticity confinement.
        let n = GRID * GRID * GRID;
        for i in 0..n {
            self.velocity[1][i] += DT * (BUOYANCY * self.heat[i] - WEIGHT * self.density[i]);
        }
        self.confine();
        // The velocity carried by itself, then made divergence-free.
        let old = self.velocity.clone();
        for (axis, field) in self.velocity.iter_mut().enumerate() {
            *field = advect(&old[axis], &old);
        }
        self.project();
        self.density = advect(&self.density, &self.velocity);
        self.heat = advect(&self.heat, &self.velocity);
        for i in 0..n {
            self.heat[i] *= COOLING;
            self.density[i] *= FADING;
        }
        self.step += 1;
    }

    /// Vorticity confinement: a force along `N × ω`, `N` the unit gradient of the vorticity's
    /// length, in the cells off the box's faces.
    fn confine(&mut self) {
        let n = GRID * GRID * GRID;
        let v = &self.velocity;
        let mut omega = vec![[0.0_f32; 3]; n];
        let mut length = vec![0.0_f32; n];
        for z in 1..GRID - 1 {
            for y in 1..GRID - 1 {
                for x in 1..GRID - 1 {
                    let d = |f: &Field, ax: usize| {
                        let (p, m) = match ax {
                            0 => (index(x + 1, y, z), index(x - 1, y, z)),
                            1 => (index(x, y + 1, z), index(x, y - 1, z)),
                            _ => (index(x, y, z + 1), index(x, y, z - 1)),
                        };
                        0.5 * (f[p] - f[m])
                    };
                    let w = [
                        d(&v[2], 1) - d(&v[1], 2),
                        d(&v[0], 2) - d(&v[2], 0),
                        d(&v[1], 0) - d(&v[0], 1),
                    ];
                    let i = index(x, y, z);
                    omega[i] = w;
                    length[i] = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
                }
            }
        }
        for z in 2..GRID - 2 {
            for y in 2..GRID - 2 {
                for x in 2..GRID - 2 {
                    let g = [
                        0.5 * (length[index(x + 1, y, z)] - length[index(x - 1, y, z)]),
                        0.5 * (length[index(x, y + 1, z)] - length[index(x, y - 1, z)]),
                        0.5 * (length[index(x, y, z + 1)] - length[index(x, y, z - 1)]),
                    ];
                    let g_len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt() + 1e-6;
                    let nn = g.map(|c| c / g_len);
                    let i = index(x, y, z);
                    let w = omega[i];
                    let f = [
                        nn[1] * w[2] - nn[2] * w[1],
                        nn[2] * w[0] - nn[0] * w[2],
                        nn[0] * w[1] - nn[1] * w[0],
                    ];
                    for (axis, field) in self.velocity.iter_mut().enumerate() {
                        field[i] += DT * CONFINEMENT * f[axis];
                    }
                }
            }
        }
    }

    /// Makes the velocity divergence-free: the pressure whose gradient takes the divergence
    /// out, by Jacobi iterations, the box's faces open (the pressure 0 beyond them).
    fn project(&mut self) {
        let n = GRID * GRID * GRID;
        let v = &self.velocity;
        let get = |f: &Field, x: isize, y: isize, z: isize| -> f32 {
            let g = GRID as isize;
            if x < 0 || y < 0 || z < 0 || x >= g || y >= g || z >= g {
                0.0
            } else {
                f[index(x as usize, y as usize, z as usize)]
            }
        };
        let mut divergence = vec![0.0; n];
        for z in 0..GRID as isize {
            for y in 0..GRID as isize {
                for x in 0..GRID as isize {
                    divergence[index(x as usize, y as usize, z as usize)] = 0.5
                        * (get(&v[0], x + 1, y, z) - get(&v[0], x - 1, y, z)
                            + get(&v[1], x, y + 1, z)
                            - get(&v[1], x, y - 1, z)
                            + get(&v[2], x, y, z + 1)
                            - get(&v[2], x, y, z - 1));
                }
            }
        }
        let mut pressure = vec![0.0; n];
        let mut next = vec![0.0; n];
        for _ in 0..PRESSURE_ITERATIONS {
            for z in 0..GRID as isize {
                for y in 0..GRID as isize {
                    for x in 0..GRID as isize {
                        let sum = get(&pressure, x + 1, y, z)
                            + get(&pressure, x - 1, y, z)
                            + get(&pressure, x, y + 1, z)
                            + get(&pressure, x, y - 1, z)
                            + get(&pressure, x, y, z + 1)
                            + get(&pressure, x, y, z - 1);
                        let i = index(x as usize, y as usize, z as usize);
                        next[i] = (sum - (divergence[i] - EXPANSION * self.heat[i])) / 6.0;
                    }
                }
            }
            std::mem::swap(&mut pressure, &mut next);
        }
        for z in 0..GRID as isize {
            for y in 0..GRID as isize {
                for x in 0..GRID as isize {
                    let i = index(x as usize, y as usize, z as usize);
                    self.velocity[0][i] -=
                        0.5 * (get(&pressure, x + 1, y, z) - get(&pressure, x - 1, y, z));
                    self.velocity[1][i] -=
                        0.5 * (get(&pressure, x, y + 1, z) - get(&pressure, x, y - 1, z));
                    self.velocity[2][i] -=
                        0.5 * (get(&pressure, x, y, z + 1) - get(&pressure, x, y, z - 1));
                }
            }
        }
    }
}

/// `f` carried by `velocity` over a step: each cell takes the value where its contents came
/// from (semi-Lagrangian, trilinear).
fn advect(f: &Field, velocity: &[Field; 3]) -> Field {
    let mut out = vec![0.0; f.len()];
    for z in 0..GRID {
        for y in 0..GRID {
            for x in 0..GRID {
                let i = index(x, y, z);
                let from = [
                    x as f32 - DT * velocity[0][i],
                    y as f32 - DT * velocity[1][i],
                    z as f32 - DT * velocity[2][i],
                ];
                out[i] = sample(f, from);
            }
        }
    }
    out
}

/// The six directions' lights at every cell: the transmittance through the smoke between the
/// cell and the box's side the light comes from, in the atlases' order (right, top, back,
/// left, bottom, front; the camera on the +z side, so "back" lights from −z).
fn lights(density: &Field) -> [Field; 6] {
    let n = GRID * GRID * GRID;
    let mut out: [Field; 6] = std::array::from_fn(|_| vec![0.0; n]);
    // Along each line of cells, from the lit side inwards: the optical depth before a cell, and
    // half its own.
    let run = |line: &mut dyn FnMut(usize) -> usize, out: &mut Field| {
        let mut depth = 0.0;
        for k in 0..GRID {
            let i = line(k);
            let own = EXTINCTION * density[i];
            out[i] = (-(depth + 0.5 * own)).exp();
            depth += own;
        }
    };
    for a in 0..GRID {
        for b in 0..GRID {
            // Right (+x side) and left.
            run(&mut |k| index(GRID - 1 - k, a, b), &mut out[0]);
            run(&mut |k| index(k, a, b), &mut out[3]);
            // Top (+y) and bottom.
            run(&mut |k| index(a, GRID - 1 - k, b), &mut out[1]);
            run(&mut |k| index(a, k, b), &mut out[4]);
            // Back (−z, behind the smoke from the camera) and front (+z, the camera's side).
            run(&mut |k| index(a, b, k), &mut out[2]);
            run(&mut |k| index(a, b, GRID - 1 - k), &mut out[5]);
        }
    }
    out
}

/// One frame's texels: per texel the six lights (each the light scattered towards the camera
/// over the alpha, 0..1) and the alpha.
fn render(density: &Field) -> Vec<([f32; 6], f32)> {
    let lit = lights(density);
    let mut out = Vec::with_capacity(TEXELS * TEXELS);
    let scale = GRID as f32 / TEXELS as f32;
    for py in 0..TEXELS {
        for px in 0..TEXELS {
            // The texel's middle in cells, rows top to bottom.
            let x = (px as f32 + 0.5) * scale - 0.5;
            let y = (TEXELS - 1 - py) as f32 * scale + 0.5 * scale - 0.5;
            let mut through = 1.0_f32;
            let mut sum = [0.0_f32; 6];
            // From the camera's side (+z) to the far side.
            for zi in (0..GRID).rev() {
                let p = [x, y, zi as f32];
                let a = 1.0 - (-EXTINCTION * sample(density, p)).exp();
                if a <= 0.0 {
                    continue;
                }
                for (s, light) in sum.iter_mut().zip(&lit) {
                    *s += through * a * sample(light, p);
                }
                through *= 1.0 - a;
            }
            let alpha = 1.0 - through;
            let lights = if alpha > 1e-4 {
                sum.map(|s| (s / alpha).clamp(0.0, 1.0))
            } else {
                [0.0; 6]
            };
            out.push((lights, alpha));
        }
    }
    out
}

/// The flipbook's two atlases, from the derived cache when there ([`crate::textures::keep_in`]),
/// else baked: keyed by this file's code and the sizes.
pub fn flipbook() -> Vec<TextureData> {
    let code = include_str!("smoke_bake.rs")
        .bytes()
        .filter(|&b| b != b'\r')
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    crate::textures::kept_by(
        "texture-smoke-flipbook",
        &(GRID, FRAMES, COLUMNS, TEXELS),
        code,
        || bake().to_vec(),
    )
}

/// The flipbook's two atlases (positive, negative), level 0 then each half to [`TEXELS`] / 8.
pub fn bake() -> [TextureData; 2] {
    let mut smoke = Smoke::new();
    let side = COLUMNS * TEXELS;
    let mut positive = vec![0_u8; side * side * 4];
    let mut negative = vec![0_u8; side * side * 4];
    let unorm = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for frame in 0..FRAMES {
        for _ in 0..STEPS_PER_FRAME {
            smoke.advance();
        }
        let texels = render(&smoke.density);
        let (cx, cy) = (frame % COLUMNS, frame / COLUMNS);
        for py in 0..TEXELS {
            for px in 0..TEXELS {
                let (l, alpha) = texels[py * TEXELS + px];
                let at = 4 * ((cy * TEXELS + py) * side + cx * TEXELS + px);
                positive[at..at + 4].copy_from_slice(&[
                    unorm(l[0].sqrt()),
                    unorm(l[1].sqrt()),
                    unorm(l[2].sqrt()),
                    unorm(alpha),
                ]);
                negative[at..at + 4].copy_from_slice(&[
                    unorm(l[3].sqrt()),
                    unorm(l[4].sqrt()),
                    unorm(l[5].sqrt()),
                    255,
                ]);
            }
        }
    }
    let levels = |base: Vec<u8>| {
        let mut levels = vec![base];
        let mut size = side;
        while size > side / 8 {
            let half = size / 2;
            let up = levels.last().expect("a level");
            let mut down = vec![0_u8; half * half * 4];
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let s: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .iter()
                            .map(|(dx, dy)| {
                                u32::from(up[4 * ((2 * y + dy) * size + 2 * x + dx) + c])
                            })
                            .sum();
                        down[4 * (y * half + x) + c] = ((s + 2) / 4) as u8;
                    }
                }
            }
            levels.push(down);
            size = half;
        }
        levels
    };
    let texture = |name: &'static str, data: Vec<u8>| TextureData {
        name: name.into(),
        size: side as u32,
        height: side as u32,
        srgb: false,
        levels: levels(data),
    };
    [
        texture("smoke flipbook +", positive),
        texture("smoke flipbook -", negative),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_puff_rises_and_the_frames_are_lit_from_where_the_light_comes() {
        let mut smoke = Smoke::new();
        for _ in 0..STEPS_PER_FRAME * 12 {
            smoke.advance();
        }
        // The smoke's middle has risen above the foot it was fed at.
        let (mut mass, mut moment) = (0.0, 0.0);
        for z in 0..GRID {
            for y in 0..GRID {
                for x in 0..GRID {
                    let d = smoke.density[index(x, y, z)];
                    mass += d;
                    moment += d * y as f32;
                }
            }
        }
        assert!(mass > 1.0, "{mass}");
        assert!(moment / mass > 0.25 * GRID as f32, "{}", moment / mass);
        // A frame: some texels opaque, the edges clear; lit from above more than from below
        // on average (the smoke's underside is in its own shadow from above, its top in its
        // shadow from below: alike), and every light in 0..1.
        let texels = render(&smoke.density);
        let opaque = texels.iter().filter(|(_, a)| *a > 0.5).count();
        assert!(opaque > 100, "{opaque}");
        assert!(texels[0].1 < 0.01 && texels[TEXELS - 1].1 < 0.01);
        assert!(
            texels
                .iter()
                .all(|(l, _)| l.iter().all(|v| (0.0..=1.0).contains(v)))
        );
    }

    /// `FORGE_SMOKE_DUMP=DIR cargo test --release -p forge-render smoke_bake -- --ignored`:
    /// the two atlases as PNGs, to look at.
    #[test]
    #[ignore = "writes images: run by hand"]
    fn dump_the_atlases() {
        let Some(dir) = std::env::var_os("FORGE_SMOKE_DUMP") else {
            return;
        };
        let side = (COLUMNS * TEXELS) as u32;
        let baked = bake();
        // A preview: lit from above and the right, over black.
        let (p, n) = (&baked[0].levels[0], &baked[1].levels[0]);
        let preview: Vec<u8> = (0..p.len() / 4)
            .flat_map(|i| {
                let light = |v: u8| (f32::from(v) / 255.0).powi(2);
                let lit =
                    0.6 * light(p[4 * i + 1]) + 0.3 * light(p[4 * i]) + 0.1 * light(n[4 * i + 2]);
                let a = f32::from(p[4 * i + 3]) / 255.0;
                let v = ((0.15 + 0.85 * lit) * a * 255.0) as u8;
                [v, v, v, 255]
            })
            .collect();
        image::save_buffer(
            std::path::Path::new(&dir).join("smoke-preview.png"),
            &preview,
            side,
            side,
            image::ExtendedColorType::Rgba8,
        )
        .expect("written");
        for (texture, name) in baked
            .iter()
            .zip(["smoke-positive.png", "smoke-negative.png"])
        {
            image::save_buffer(
                std::path::Path::new(&dir).join(name),
                &texture.levels[0],
                side,
                side,
                image::ExtendedColorType::Rgba8,
            )
            .expect("written");
        }
    }

    #[test]
    fn the_bake_is_the_same_every_time() {
        let a = bake();
        let b = bake();
        assert_eq!(a[0].levels, b[0].levels);
        assert_eq!(a[1].levels[0], b[1].levels[0]);
        assert_eq!(a[0].levels.len(), 4);
        assert_eq!(a[0].levels[0].len(), (COLUMNS * TEXELS).pow(2) * 4);
    }
}

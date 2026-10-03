//! Buoyancy and drag on floating hulls (Phase 3's step 3, issue #138; D-009: "boats by
//! submerged-triangle hydrostatics"), after Jacques Kerner, "Water interaction model for boats
//! in video games" (2015): each triangle of a closed hull is cut where the water's surface
//! crosses it, the pressure at its depth pushes each submerged piece along its normal, and the
//! water it moves through drags it.
//!
//! All of it is plain `f32`/`f64` arithmetic with no transcendental function: the same inputs
//! give the same force on every platform, so a server and its clients float a boat alike.

use glam::{DVec3, Vec3};

use crate::{Transform, Velocity};

/// A closed hull in its body's frame: the triangles, wound counter-clockwise seen from
/// outside, and the volume they hold.
#[derive(Clone, Debug)]
pub struct Hull {
    vertices: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    volume: f32,
}

impl Hull {
    /// A hull from a closed mesh in its body's frame.
    pub fn new(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Self {
        // The divergence theorem: the signed volumes of the tetrahedra to the origin.
        let volume = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| vertices[i as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum::<f32>();
        Self {
            vertices: vertices.to_vec(),
            triangles: triangles.to_vec(),
            volume,
        }
    }

    /// A box of the given half sizes, centred on the origin, each face cut into `n` × `n`
    /// squares (finer faces cut more truly at the water line).
    pub fn cuboid(half: Vec3, n: u32) -> Self {
        let n = n.max(1);
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for (axis, sign) in [
            (0, 1.0),
            (0, -1.0),
            (1, 1.0),
            (1, -1.0),
            (2, 1.0),
            (2, -1.0),
        ] {
            let (u, v) = if sign > 0.0 {
                ((axis + 1) % 3, (axis + 2) % 3)
            } else {
                ((axis + 2) % 3, (axis + 1) % 3)
            };
            let first = vertices.len() as u32;
            for j in 0..=n {
                for i in 0..=n {
                    let mut p = Vec3::ZERO;
                    p[axis] = sign * half[axis];
                    p[u] = half[u] * (2.0 * i as f32 / n as f32 - 1.0);
                    p[v] = half[v] * (2.0 * j as f32 / n as f32 - 1.0);
                    vertices.push(p);
                }
            }
            let at = |i: u32, j: u32| first + j * (n + 1) + i;
            for j in 0..n {
                for i in 0..n {
                    triangles.push([at(i, j), at(i + 1, j), at(i + 1, j + 1)]);
                    triangles.push([at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
                }
            }
        }
        Self::new(&vertices, &triangles)
    }

    /// A cylinder along +y from `bottom` to `bottom + length`, `around` sides, cut into `along`
    /// rings (a barrel, a log stood up and turned by its body).
    pub fn cylinder(radius: f32, length: f32, bottom: f32, around: u32, along: u32) -> Self {
        let around = around.max(3);
        let along = along.max(1);
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        // A unit circle without trigonometry: the polygon's corners from rotations by a fixed
        // step, the step's cosine and sine from the half-angle formulas of a right angle.
        let corners = circle(around);
        for k in 0..=along {
            let y = bottom + length * k as f32 / along as f32;
            for &(c, s) in &corners {
                vertices.push(Vec3::new(radius * c, y, radius * s));
            }
        }
        let ring = |k: u32, i: u32| k * around + i % around;
        for k in 0..along {
            for i in 0..around {
                let (a, b) = (ring(k, i), ring(k, i + 1));
                let (c, d) = (ring(k + 1, i), ring(k + 1, i + 1));
                triangles.push([a, c, b]);
                triangles.push([b, c, d]);
            }
        }
        // The caps, fans round a centre each.
        let bottom_centre = vertices.len() as u32;
        vertices.push(Vec3::new(0.0, bottom, 0.0));
        let top_centre = vertices.len() as u32;
        vertices.push(Vec3::new(0.0, bottom + length, 0.0));
        for i in 0..around {
            triangles.push([bottom_centre, ring(0, i), ring(0, i + 1)]);
            triangles.push([top_centre, ring(along, i + 1), ring(along, i)]);
        }
        Self::new(&vertices, &triangles)
    }

    /// A ball of `radius` round `centre`: a cube of `n` × `n` squares a face pushed out onto
    /// the sphere (no trigonometry).
    pub fn sphere(radius: f32, centre: Vec3, n: u32) -> Self {
        let cube = Self::cuboid(Vec3::ONE, n);
        let vertices: Vec<Vec3> = cube
            .vertices
            .iter()
            .map(|v| centre + v.normalize() * radius)
            .collect();
        Self::new(&vertices, &cube.triangles)
    }

    /// The volume it holds, m³.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Its vertices, in its body's frame.
    pub fn vertices(&self) -> &[Vec3] {
        &self.vertices
    }

    /// Triangles.
    pub fn triangles(&self) -> usize {
        self.triangles.len()
    }
}

/// `n` points round the unit circle as (cos, sin), from (1, 0), by repeated rotation through
/// a step whose cosine and sine come from their Taylor series: no platform `sin`, so the same
/// bits everywhere.
fn circle(n: u32) -> Vec<(f32, f32)> {
    let n = n as usize;
    let step = std::f64::consts::TAU / n as f64;
    // Twelve terms: exact to f64 for a step of at most a third of a turn.
    let (mut s, mut c) = (0.0f64, 0.0f64);
    let (mut term_s, mut term_c) = (step, 1.0);
    for k in 0..12 {
        s += term_s;
        c += term_c;
        let k = k as f64;
        term_s *= -step * step / ((2.0 * k + 2.0) * (2.0 * k + 3.0));
        term_c *= -step * step / ((2.0 * k + 1.0) * (2.0 * k + 2.0));
    }
    let mut out = Vec::with_capacity(n);
    let (mut x, mut y) = (1.0f64, 0.0f64);
    for _ in 0..n {
        out.push((x as f32, y as f32));
        (x, y) = (x * c - y * s, x * s + y * c);
    }
    out
}

/// The water a hull floats in.
pub trait Water {
    /// The surface's height at world (x, z), metres.
    fn height(&self, x: f64, z: f64) -> f64;
    /// The water's own velocity there (a current, a river's flow), m/s.
    fn current(&self, _x: f64, _z: f64) -> Vec3 {
        Vec3::ZERO
    }
}

/// Still water at a level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level(pub f64);

impl Water for Level {
    fn height(&self, _x: f64, _z: f64) -> f64 {
        self.0
    }
}

/// How deep a face may lie and still make waves as it moves, metres.
const RADIATION_DEPTH: f32 = 2.0;

/// The water's density and how it drags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fluid {
    /// kg/m³: 1025 for the sea, 1000 for fresh water.
    pub density: f32,
    /// m/s², downwards.
    pub gravity: f32,
    /// Pressure drag on the faces moving into the water (a flat plate's is about 1).
    pub pressure_drag: f32,
    /// Friction along the wetted faces.
    pub skin_drag: f32,
    /// The waves a body makes as it heaves and rolls carry its energy away (radiation
    /// damping): a pull against the motion of each wetted face across itself, in proportion to
    /// its speed and its area, m/s.
    pub radiation: f32,
}

impl Fluid {
    /// The sea.
    pub const SEA: Self = Self {
        density: 1025.0,
        gravity: 9.81,
        pressure_drag: 0.6,
        skin_drag: 0.01,
        radiation: 0.5,
    };
}

/// What the water does to a hull this step: a force through the body's origin and a torque
/// about it, and how much of it is under.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Push {
    /// N.
    pub force: Vec3,
    /// N·m, about the body's origin.
    pub torque: Vec3,
    /// The hull's area under the water, m².
    pub wetted: f32,
    /// The buoyancy alone, N (upwards when afloat).
    pub buoyancy: Vec3,
}

/// The water's push on `hull` at `at`, moving at `velocity` (its centre of mass's, with the
/// spin about `center_of_mass`).
pub fn push(
    hull: &Hull,
    at: Transform,
    velocity: Velocity,
    center_of_mass: DVec3,
    water: &impl Water,
    fluid: &Fluid,
) -> Push {
    // Offsets from the body's origin, in the world's axes, and the depth under the surface of
    // each vertex (positive under).
    let offsets: Vec<Vec3> = hull.vertices.iter().map(|&v| at.rotation * v).collect();
    let depths: Vec<f32> = offsets
        .iter()
        .map(|&r| {
            let p = at.position + r.as_dvec3();
            (water.height(p.x, p.z) - p.y) as f32
        })
        .collect();
    let com = (center_of_mass - at.position).as_vec3();
    let mut out = Push::default();
    let rho_g = fluid.density * fluid.gravity;
    for t in &hull.triangles {
        let [a, b, c] = t.map(|i| i as usize);
        let (da, db, dc) = (depths[a], depths[b], depths[c]);
        if da <= 0.0 && db <= 0.0 && dc <= 0.0 {
            continue;
        }
        let points = [(offsets[a], da), (offsets[b], db), (offsets[c], dc)];
        let mut piece = [(Vec3::ZERO, 0.0f32); 4];
        let n = clip_under(&points, &mut piece);
        for k in 1..n.saturating_sub(1) {
            let (p0, d0) = piece[0];
            let (p1, d1) = piece[k];
            let (p2, d2) = piece[k + 1];
            let area = 0.5 * (p1 - p0).cross(p2 - p0);
            let size = area.length();
            if size <= 1e-9 {
                continue;
            }
            let normal = area / size;
            let centre = (p0 + p1 + p2) / 3.0;
            let depth = (d0 + d1 + d2) / 3.0;
            // The pressure at its depth, inwards.
            let pressure = -rho_g * depth * area;
            // The water past it: its velocity there against the current.
            let world = at.position + centre.as_dvec3();
            let v = velocity.linear + velocity.angular.cross(centre - com)
                - water.current(world.x, world.z);
            let vn = v.dot(normal);
            let mut drag = Vec3::ZERO;
            if vn > 0.0 {
                drag -= 0.5 * fluid.density * fluid.pressure_drag * size * vn * vn * normal;
            }
            let tangent = v - vn * normal;
            drag -= 0.5 * fluid.density * fluid.skin_drag * size * tangent.length() * tangent;
            // Only near the surface, where the motion makes waves: none from 2 m down.
            let near = (1.0 - depth / RADIATION_DEPTH).clamp(0.0, 1.0);
            drag -= fluid.density * fluid.radiation * near * size * vn * normal;
            let force = pressure + drag;
            out.force += force;
            out.torque += centre.cross(force);
            out.buoyancy += pressure;
            out.wetted += size;
        }
    }
    out
}

/// The part of a triangle under the water (depth above zero), as a polygon of up to four
/// corners with their depths; returns the corners written.
fn clip_under(points: &[(Vec3, f32); 3], out: &mut [(Vec3, f32); 4]) -> usize {
    let mut n = 0;
    for i in 0..3 {
        let (p, d) = points[i];
        let (q, e) = points[(i + 1) % 3];
        if d > 0.0 {
            out[n] = (p, d);
            n += 1;
        }
        if (d > 0.0) != (e > 0.0) {
            let t = d / (d - e);
            out[n] = (p + (q - p) * t, 0.0);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn hulls_hold_their_volumes() {
        let cube = Hull::cuboid(Vec3::new(0.5, 0.25, 1.0), 3);
        assert!((cube.volume() - 1.0).abs() < 1e-5, "{}", cube.volume());
        let can = Hull::cylinder(0.3, 0.88, 0.0, 32, 2);
        // A 32-gon's area is 0.9936 of the circle's.
        let expected = 0.3f32 * 0.3 * std::f32::consts::PI * 0.88 * 0.993_6;
        assert!((can.volume() - expected).abs() < 1e-3, "{}", can.volume());
        let ball = Hull::sphere(0.25, Vec3::new(0.0, 0.25, 0.0), 6);
        let sphere = 4.0 / 3.0 * std::f32::consts::PI * 0.25f32.powi(3);
        // Its flat faces lie inside the sphere: 3 % short at six squares a face.
        assert!(
            (ball.volume() / sphere - 1.0).abs() < 0.05,
            "{}",
            ball.volume()
        );
    }

    #[test]
    fn a_box_under_water_is_pushed_up_by_the_water_it_moves_aside() {
        let cube = Hull::cuboid(Vec3::splat(0.5), 4);
        let at = Transform {
            position: DVec3::new(10.0, -3.0, 5.0),
            rotation: Quat::from_rotation_y(0.3),
        };
        let p = push(
            &cube,
            at,
            Velocity::default(),
            at.position,
            &Level(0.0),
            &Fluid::SEA,
        );
        let weight_of_water = 1025.0 * 9.81 * 1.0;
        assert!((p.force.y - weight_of_water).abs() < 1.0, "{}", p.force);
        assert!(
            p.force.x.abs() < 1e-2 && p.force.z.abs() < 1e-2,
            "{}",
            p.force
        );
        assert!(p.torque.length() < 1e-1, "{}", p.torque);
    }

    #[test]
    fn a_raft_half_under_is_pushed_by_half_its_volume_and_rights_itself() {
        // A raft 2 m square and 0.2 m thick: wide enough to float flat (a cube half under
        // would not: its metacentre is under its centre of mass).
        let cube = Hull::cuboid(Vec3::new(1.0, 0.1, 1.0), 4);
        let at = Transform {
            position: DVec3::new(0.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
        };
        let p = push(
            &cube,
            at,
            Velocity::default(),
            at.position,
            &Level(0.0),
            &Fluid::SEA,
        );
        assert!((p.force.y - 0.4 * 1025.0 * 9.81).abs() < 1.0, "{}", p.force);
        // Tilted, the pushed side turns it back upright.
        let tilted = Transform {
            rotation: Quat::from_rotation_z(0.1),
            ..at
        };
        let p = push(
            &cube,
            tilted,
            Velocity::default(),
            at.position,
            &Level(0.0),
            &Fluid::SEA,
        );
        assert!(p.torque.z < 0.0, "a righting torque: {}", p.torque);
    }

    #[test]
    fn moving_through_water_is_slowed() {
        let cube = Hull::cuboid(Vec3::splat(0.5), 4);
        let at = Transform {
            // Deep enough to make no waves.
            position: DVec3::new(0.0, -5.0, 0.0),
            rotation: Quat::IDENTITY,
        };
        let moving = Velocity {
            linear: Vec3::new(2.0, 0.0, 0.0),
            angular: Vec3::ZERO,
        };
        let p = push(&cube, at, moving, at.position, &Level(0.0), &Fluid::SEA);
        // The front face, a square metre at 2 m/s: half the density times 0.6 times 4; and the
        // four faces along the way: half the density times 0.01 times 2 times 2 each.
        let expected = 0.5 * 1025.0 * (0.6 * 4.0 + 4.0 * 0.01 * 4.0);
        assert!((p.force.x + expected).abs() < 5.0, "{}", p.force);
    }
}

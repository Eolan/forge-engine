//! The slime in `physics-lab --lab creatures` (#179, #180): the tropical island's slime (its
//! `meshgen::slime`, the owner's pick), a squat drop of mint jelly with a darker nucleus floating
//! inside and two tall glossy eyes, as a Jolt soft body that hops before the dogs.
//!
//! - **The body:** 258 points (an octahedron cut three times into four, then shaped into the
//!   drop: its underside flattened, its sides bulging), held by their edges, their bends and the
//!   pressure inside. Soft enough to sag where it sits and to squash and wobble as it lands.
//! - **The drawing:** a skinned mesh whose joints are those points. The drawn drop is finer
//!   (4 098 vertices, the same octahedron cut twice more), each of its vertices weighted between
//!   the three points of the triangle it lies on, and each point's matrix takes it from its place
//!   at rest to where it is, turned as the surface about it turned ([`skin`]). The eyes
//!   (section 1) and the nucleus (section 2) ride the surface the same way.
//! - **The jelly:** the renderer's jelly class (D-051): the scene behind it seen through it, bent
//!   and tinted, the nucleus and the eyes' backs met inside it, the sun glowing through it.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::Result;
use forge_geom::{TriMesh, VertexSkin};
use forge_physics::{BodyId, SoftBodyDesc, World};
use glam::{DVec3, Mat4, Quat, Vec3};

/// The drop's shape at rest, from the tropical island's slime scaled by 0.6: its half width,
/// its height over its middle, how much flatter its underside is (0.3: the bottom stands 0.3 of
/// the height under the middle) and how far its sides bulge.
const HALF_WIDTH: f32 = 0.3;
const HEIGHT: f32 = 0.276;
const FLAT: f32 = 0.3;
const BULGE: f32 = 0.12;
/// Its mass, kilograms.
const MASS: f32 = 2.5;
/// How far its edges and bends give (Jolt's compliance, m/N), and the pressure inside it
/// (n R T): soft enough to sag a little where it sits and to squash and wobble as it lands.
const COMPLIANCE: f32 = 1.0e-3;
const BEND_COMPLIANCE: f32 = 5.0e-2;
const PRESSURE: f32 = 8.0;
/// Where its middle starts: before the dogs, at the camera's feet, its bottom 2 cm up.
const AT: DVec3 = DVec3::new(-0.2, (FLAT * HEIGHT) as f64 + 0.02, 2.3);
/// A hop every `HOP` ticks: the speed it takes up, and along its way, m/s, its way turning a
/// quarter each hop (a square it comes back round).
const HOP: u64 = 75;
const HOP_UP: f32 = 2.8;
const HOP_ALONG: f32 = 0.8;
const WAYS: [[f32; 2]; 4] = [[1.0, 0.0], [0.0, -1.0], [-1.0, 0.0], [0.0, 1.0]];
/// The sphere about its middle its points stay in, however it squashes, metres.
pub(super) const BOUND: f32 = 1.0;
/// Its points (the skinned mesh's joints).
pub(super) const POINTS: usize = 258;

/// The eyes, facing the camera at the start (+z): the directions from the middle they stand on,
/// and their half sizes across, up and out of the surface, metres. They are set into the jelly,
/// a third of them standing out.
const EYES: [[f32; 3]; 2] = [[-0.33, 0.45, 0.83], [0.33, 0.45, 0.83]];
const EYE: [f32; 3] = [0.028, 0.051, 0.033];
/// The nucleus: its middle and half sizes, metres (a little back from the middle, as the
/// island's).
const NUCLEUS_AT: [f32; 3] = [0.018, 0.04, -0.03];
const NUCLEUS: [f32; 3] = [0.09, 0.072, 0.084];
/// What the nucleus's vertices are weighted on, all alike: the four points round the drop's
/// waist (the octahedron's ±x and ±z), so it floats as one piece, carried and turned as the
/// body's middle is, while the points under it would swing it apart as the bottom flattens.
const WAIST: [(u32, f32); 4] = [(0, 0.25), (1, 0.25), (4, 0.25), (5, 0.25)];

/// The slime's body.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slime {
    pub body: BodyId,
}

/// The slime's surfaces: its points as the soft body takes them, and the finer mesh drawn with
/// its skin.
pub(super) struct Surface {
    /// Its points at rest about its middle, and its triangles, wound outwards.
    pub points: Vec<Vec3>,
    pub faces: Vec<[u32; 3]>,
    /// Its points' normals at rest, unit (their triangles').
    pub normals: Vec<Vec3>,
    /// The drawn drop, its eyes (section 1) and its nucleus (section 2), and each vertex's
    /// points.
    pub mesh: TriMesh,
    pub skin: Vec<VertexSkin>,
}

/// The drop at the unit direction `d` from its middle: the tropical island's shape, the
/// underside flattened and the sides bulging.
fn shape(d: Vec3) -> Vec3 {
    let y = if d.y < 0.0 { d.y * FLAT } else { d.y };
    let bulge = 1.0 + BULGE * (1.0 - d.y.abs());
    Vec3::new(
        d.x * HALF_WIDTH * bulge,
        y * HEIGHT,
        d.z * HALF_WIDTH * bulge,
    )
}

/// The drop's outward normal at the unit direction `d`, from its shape's change about it.
fn shape_normal(d: Vec3) -> Vec3 {
    let t1 = if d.y.abs() < 0.99 { Vec3::Y } else { Vec3::X }
        .cross(d)
        .normalize();
    let t2 = d.cross(t1);
    let e = 1e-3;
    let du = shape((d + t1 * e).normalize()) - shape((d - t1 * e).normalize());
    let dv = shape((d + t2 * e).normalize()) - shape((d - t2 * e).normalize());
    let n = du.cross(dv).normalize();
    if n.dot(shape(d)) < 0.0 { -n } else { n }
}

/// A ball of unit radius being cut: its points, triangles, and each point's weights on the
/// points of the ball it was cut from.
struct Ball {
    points: Vec<Vec3>,
    faces: Vec<[u32; 3]>,
    weights: Vec<Vec<(u32, f32)>>,
}

impl Ball {
    fn octahedron() -> Self {
        let points = vec![Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        let weights = (0..6).map(|k| vec![(k, 1.0)]).collect();
        let faces = vec![
            [0, 2, 4],
            [4, 2, 1],
            [1, 2, 5],
            [5, 2, 0],
            [0, 4, 3],
            [4, 1, 3],
            [1, 5, 3],
            [5, 0, 3],
        ];
        Self {
            points,
            faces,
            weights,
        }
    }

    /// Every triangle cut into four, the new points pushed onto the sphere and weighted half
    /// and half on their edge's.
    fn cut(&mut self) {
        let mut middles = HashMap::new();
        let mut faces = Vec::with_capacity(4 * self.faces.len());
        for [a, b, c] in std::mem::take(&mut self.faces) {
            let ab = self.middle(&mut middles, a, b);
            let bc = self.middle(&mut middles, b, c);
            let ca = self.middle(&mut middles, c, a);
            faces.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        self.faces = faces;
    }

    fn middle(&mut self, middles: &mut HashMap<(u32, u32), u32>, a: u32, b: u32) -> u32 {
        *middles.entry((a.min(b), a.max(b))).or_insert_with(|| {
            let (a, b) = (a as usize, b as usize);
            self.points
                .push((self.points[a] + self.points[b]).normalize());
            let mut w: Vec<(u32, f32)> = Vec::new();
            for &(j, x) in self.weights[a].iter().chain(&self.weights[b]) {
                match w.iter_mut().find(|(k, _)| *k == j) {
                    Some((_, y)) => *y += 0.5 * x,
                    None => w.push((j, 0.5 * x)),
                }
            }
            self.weights.push(w);
            self.points.len() as u32 - 1
        })
    }
}

/// The slime's surfaces, made once.
pub(super) fn surface() -> &'static Surface {
    static SURFACE: OnceLock<Surface> = OnceLock::new();
    SURFACE.get_or_init(|| {
        let mut ball = Ball::octahedron();
        for _ in 0..3 {
            ball.cut();
        }
        let points: Vec<Vec3> = ball.points.iter().map(|&p| shape(p)).collect();
        let faces = ball.faces.clone();
        // From here on, every point is weighted on the soft body's.
        ball.weights = (0..points.len() as u32).map(|k| vec![(k, 1.0)]).collect();
        for _ in 0..2 {
            ball.cut();
        }
        let positions: Vec<Vec3> = ball.points.iter().map(|&p| shape(p)).collect();
        let mut mesh = TriMesh {
            normals: normals(&ball.faces, &positions)
                .into_iter()
                .map(|n| n.normalize().to_array())
                .collect(),
            positions: positions.iter().map(|p| p.to_array()).collect(),
            indices: ball.faces.iter().flatten().copied().collect(),
            sections: vec![0; ball.faces.len()],
            uvs: Vec::new(),
        };
        let mut skin: Vec<VertexSkin> = ball.weights.iter().map(|w| vertex_skin(w)).collect();
        let mut parts = Parts {
            points: &points,
            faces: &faces,
            mesh: &mut mesh,
            skin: &mut skin,
        };
        for eye in EYES {
            let d = Vec3::from_array(eye).normalize();
            let out = shape_normal(d);
            let across = Vec3::Y.cross(out).normalize();
            let up = out.cross(across);
            let middle = shape(d) - out * (EYE[2] / 3.0);
            parts.ellipsoid(middle, [across, up, out], EYE, 1, (8, 16), None);
        }
        parts.ellipsoid(
            Vec3::from_array(NUCLEUS_AT),
            [Vec3::X, Vec3::Y, Vec3::Z],
            NUCLEUS,
            2,
            (16, 24),
            Some(&WAIST),
        );
        assert_eq!(points.len(), POINTS);
        Surface {
            normals: normals(&faces, &points)
                .into_iter()
                .map(Vec3::normalize)
                .collect(),
            points,
            faces,
            mesh,
            skin,
        }
    })
}

/// A vertex's skin from its weights on the soft body's points (at most four).
fn vertex_skin(w: &[(u32, f32)]) -> VertexSkin {
    assert!(w.len() <= 4, "at most four points a vertex");
    let mut skin = VertexSkin {
        joints: [0; 4],
        weights: [0.0; 4],
    };
    for (k, &(j, x)) in w.iter().enumerate() {
        skin.joints[k] = j as u16;
        skin.weights[k] = x;
    }
    skin
}

/// The parts added to the drawn drop, each vertex weighted on the soft body's points as the
/// point of the surface in its direction is, so the parts ride the surface as it squashes.
struct Parts<'a> {
    points: &'a [Vec3],
    faces: &'a [[u32; 3]],
    mesh: &'a mut TriMesh,
    skin: &'a mut Vec<VertexSkin>,
}

impl Parts<'_> {
    /// An ellipsoid about `middle` with half sizes `half` along the frame's axes (right-handed),
    /// in `section`, of `(rings, around)` quads: rings from the third axis's pole round to the
    /// other. Its vertices take `weights` when given (the same for all: the part moves as one
    /// piece), else the surface's under them.
    #[allow(clippy::too_many_arguments)]
    fn ellipsoid(
        &mut self,
        middle: Vec3,
        axes: [Vec3; 3],
        half: [f32; 3],
        section: u8,
        (rings, around): (u32, u32),
        weights: Option<&[(u32, f32)]>,
    ) {
        let first = self.mesh.positions.len() as u32;
        for ring in 0..=rings {
            let polar = std::f32::consts::PI * ring as f32 / rings as f32;
            for k in 0..around {
                let a = std::f32::consts::TAU * k as f32 / around as f32;
                let unit = [polar.sin() * a.cos(), polar.sin() * a.sin(), polar.cos()];
                let p = middle + (0..3).map(|i| axes[i] * (half[i] * unit[i])).sum::<Vec3>();
                let n = (0..3).map(|i| axes[i] * (unit[i] / half[i])).sum::<Vec3>();
                let w = match weights {
                    Some(w) => w.to_vec(),
                    None => under(self.points, self.faces, p.normalize()),
                };
                self.skin.push(vertex_skin(&w));
                self.mesh.positions.push(p.to_array());
                self.mesh.normals.push(n.normalize().to_array());
            }
        }
        for ring in 0..rings {
            for k in 0..around {
                let a = first + ring * around + k;
                let b = first + ring * around + (k + 1) % around;
                let (c, d) = (a + around, b + around);
                self.mesh.indices.extend([a, c, b, b, c, d]);
                self.mesh.sections.extend([section, section]);
            }
        }
    }
}

/// The soft body's triangle under the direction `d` from its middle, as weights on its three
/// points (the ray's barycentrics on the flat triangle).
fn under(points: &[Vec3], faces: &[[u32; 3]], d: Vec3) -> Vec<(u32, f32)> {
    for &[a, b, c] in faces {
        let [pa, pb, pc] = [a, b, c].map(|v| points[v as usize]);
        // Möller–Trumbore from the middle along `d`.
        let (e1, e2) = (pb - pa, pc - pa);
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = -pa;
        let u = s.dot(h) / det;
        let q = s.cross(e1);
        let v = d.dot(q) / det;
        let t = e2.dot(q) / det;
        if t > 0.0 && u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6 {
            let (u, v) = (u.max(0.0), v.max(0.0));
            let w = (1.0 - u - v).max(0.0);
            let sum = u + v + w;
            return vec![(a, w / sum), (b, u / sum), (c, v / sum)];
        }
    }
    unreachable!("every direction meets the closed drop")
}

/// Adds the slime to `world`.
pub(super) fn build(world: &mut World) -> Result<Slime> {
    let s = surface();
    let body = world.add_soft_body(&SoftBodyDesc {
        points: &s.points,
        faces: &s.faces,
        position: AT,
        inverse_mass: POINTS as f32 / MASS,
        compliance: COMPLIANCE,
        bend_compliance: BEND_COMPLIANCE,
        pressure: PRESSURE,
        friction: 0.8,
        restitution: 0.0,
        iterations: 5,
        user_data: 0,
    })?;
    Ok(Slime { body })
}

impl Slime {
    /// Before the step of tick `tick`: a hop every [`HOP`] ticks.
    pub(super) fn drive(self, world: &mut World, tick: u64) {
        if tick % HOP == HOP - 1 {
            let [x, z] = WAYS[(tick / HOP % 4) as usize];
            world.push_soft_body(self.body, Vec3::new(HOP_ALONG * x, HOP_UP, HOP_ALONG * z));
        }
    }

    /// Its points in the world after the last step into `out`.
    pub(super) fn points(self, world: &World, out: &mut Vec<Vec3>) {
        let origin = world.soft_body_vertices(self.body, out);
        for p in out.iter_mut() {
            *p = (origin + p.as_dvec3()).as_vec3();
        }
    }
}

/// The matrices that bend the slime's mesh, one a point into `out`: each takes the point from
/// its place at rest to `points[k]` (about the slime's mover), turned as the surface's normal
/// there turned.
pub(super) fn skin(points: &[Vec3], out: &mut Vec<Mat4>) {
    let s = surface();
    out.clear();
    out.extend(
        s.points
            .iter()
            .zip(points)
            .zip(s.normals.iter().zip(normals(&s.faces, points)))
            .map(|((&rest, &at), (&was, now))| {
                let turn = Quat::from_rotation_arc(was, now.normalize_or(was));
                Mat4::from_rotation_translation(turn, at - turn * rest)
            }),
    );
}

/// Each point's normal on a surface: its triangles' summed (weighted by their areas).
fn normals(faces: &[[u32; 3]], points: &[Vec3]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; points.len()];
    for &[a, b, c] in faces {
        let [pa, pb, pc] = [a, b, c].map(|v| points[v as usize]);
        let n = (pb - pa).cross(pc - pa);
        for v in [a, b, c] {
            normals[v as usize] += n;
        }
    }
    normals
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_physics::{BodyDesc, Shape, WorldDesc};

    #[test]
    fn the_drawn_drop_is_weighted_on_the_soft_bodys_points() {
        let s = surface();
        assert_eq!(s.faces.len(), 512);
        assert_eq!(s.mesh.positions.len(), s.skin.len());
        assert_eq!(s.mesh.sections.len(), s.mesh.triangle_count());
        assert_eq!(s.mesh.sections.iter().filter(|&&k| k == 0).count(), 8192);
        for v in &s.skin {
            let sum: f32 = v.weights.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "weights summing to {sum}");
            assert!(v.joints.iter().all(|&j| (j as usize) < POINTS));
        }
        // At rest, every matrix leaves its vertex where it is.
        let mut m = Vec::new();
        skin(&s.points, &mut m);
        for (k, p) in s.mesh.positions.iter().enumerate() {
            let p = Vec3::from_array(*p);
            let w = &s.skin[k];
            let at: Vec3 = (0..4)
                .map(|i| w.weights[i] * m[w.joints[i] as usize].transform_point3(p))
                .sum();
            assert!(at.distance(p) < 1e-4, "vertex {k} moved to {at}");
        }
    }

    #[test]
    fn the_parts_wind_outwards_the_eyes_on_the_surface_the_nucleus_inside() {
        let s = surface();
        let m = &s.mesh;
        let p = |v: u32| Vec3::from_array(m.positions[v as usize]);
        let n = |v: u32| Vec3::from_array(m.normals[v as usize]);
        let mut parts = [0; 3];
        for (t, tri) in m.indices.chunks(3).enumerate() {
            let section = m.sections[t] as usize;
            parts[section] += 1;
            let g = (p(tri[1]) - p(tri[0])).cross(p(tri[2]) - p(tri[0]));
            if g.length_squared() < 1e-14 {
                continue; // a pole's
            }
            let normal = n(tri[0]) + n(tri[1]) + n(tri[2]);
            assert!(
                g.dot(normal) > 0.0,
                "triangle {t} (section {section}) winds inwards"
            );
            for &v in tri {
                let at = p(v);
                let surface = shape(at.normalize()).length();
                match section {
                    1 => assert!(
                        (at.length() - surface).abs() < 0.06,
                        "an eye's vertex {v} off the surface"
                    ),
                    2 => assert!(
                        at.length() < surface - 0.03,
                        "the nucleus's vertex {v} near the surface"
                    ),
                    _ => {}
                }
            }
        }
        assert!(parts[1] > 0 && parts[2] > 0, "{parts:?}");
    }

    #[test]
    fn the_slime_sits_squat_and_hops() {
        let mut world = World::new(&WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(10.0, 0.5, 10.0), 0.05, 1000.0).unwrap();
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .unwrap();
        let slime = build(&mut world).unwrap();
        let mut points = Vec::new();
        let span = |points: &[Vec3]| {
            points
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p.y), h.max(p.y)))
        };
        let mut highest = f32::MIN;
        for tick in 0..HOP + 30 {
            slime.drive(&mut world, tick);
            world.step(1.0 / 60.0, 1).unwrap();
            slime.points(&world, &mut points);
            if tick == HOP - 2 {
                let (low, high) = span(&points);
                eprintln!("SLIME sits {low} to {high}");
                assert!(low > -0.02 && high < 0.45, "sits {low} to {high}");
            }
            if tick >= HOP {
                highest = highest.max(span(&points).0);
            }
        }
        eprintln!("SLIME hops {highest}");
        assert!(highest > 0.2, "its bottom rose to {highest} m");
    }
}

//! The slime in `physics-lab --lab creatures` (Phase 3's step 7): a ball of jelly as a Jolt soft
//! body, its 258 points held by their edges and the pressure inside, hopping before the dogs. It
//! is drawn as a skinned mesh whose joints are its points: a finer ball (4 098 vertices), each of
//! its vertices weighted between the three points of the triangle it lies on, and each point's
//! matrix taking it from its place at rest to where it is, turned as the surface about it turned
//! ([`skin`]).

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::Result;
use forge_geom::{TriMesh, VertexSkin};
use forge_physics::{BodyId, SoftBodyDesc, World};
use glam::{DVec3, Mat4, Quat, Vec3};

/// Its radius at rest and its mass: metres, kilograms.
pub(super) const RADIUS: f32 = 0.3;
const MASS: f32 = 2.5;
/// How far its edges give (Jolt's compliance, m/N), and the pressure inside it (n R T): soft
/// enough to sag a little where it sits and to squash when it lands.
const COMPLIANCE: f32 = 5.0e-4;
const PRESSURE: f32 = 15.0;
/// Where it starts: before the dogs, at the camera's feet.
const AT: DVec3 = DVec3::new(-0.2, 0.32, 2.3);
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

/// The slime's body.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slime {
    pub body: BodyId,
}

/// The slime's surfaces: its points as the soft body takes them (an octahedron cut three times
/// into four), and the finer mesh drawn (cut twice more) with its skin.
pub(super) struct Surface {
    /// Its points at rest about its middle, and its triangles, wound outwards.
    pub points: Vec<Vec3>,
    pub faces: Vec<[u32; 3]>,
    /// Its points' normals at rest, unit (their triangles', not the sphere's).
    pub normals: Vec<Vec3>,
    /// The drawn ball and each of its vertices' points.
    pub mesh: TriMesh,
    pub skin: Vec<VertexSkin>,
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
        let points: Vec<Vec3> = ball.points.iter().map(|&p| RADIUS * p).collect();
        let faces = ball.faces.clone();
        // From here on, every point is weighted on the soft body's.
        ball.weights = (0..points.len() as u32).map(|k| vec![(k, 1.0)]).collect();
        for _ in 0..2 {
            ball.cut();
        }
        let skin = ball
            .weights
            .iter()
            .map(|w| {
                assert!(w.len() <= 3, "a vertex on one triangle");
                let mut skin = VertexSkin {
                    joints: [0; 4],
                    weights: [0.0; 4],
                };
                for (k, &(j, x)) in w.iter().enumerate() {
                    skin.joints[k] = j as u16;
                    skin.weights[k] = x;
                }
                skin
            })
            .collect();
        let mesh = TriMesh {
            positions: ball
                .points
                .iter()
                .map(|&p| (RADIUS * p).to_array())
                .collect(),
            normals: ball.points.iter().map(|p| p.to_array()).collect(),
            indices: ball.faces.iter().flatten().copied().collect(),
            sections: Vec::new(),
            uvs: Vec::new(),
        };
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

/// Adds the slime to `world`.
pub(super) fn build(world: &mut World) -> Result<Slime> {
    let s = surface();
    let body = world.add_soft_body(&SoftBodyDesc {
        points: &s.points,
        faces: &s.faces,
        position: AT,
        inverse_mass: POINTS as f32 / MASS,
        compliance: COMPLIANCE,
        bend_compliance: f32::MAX,
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
    fn the_drawn_ball_is_weighted_on_the_soft_bodys_points() {
        let s = surface();
        assert_eq!(s.faces.len(), 512);
        assert_eq!(s.mesh.positions.len(), 4098);
        assert_eq!(s.skin.len(), 4098);
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
    fn the_slime_sits_squashed_and_hops() {
        let mut world = World::new(&WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(10.0, 0.5, 10.0), 0.05, 1000.0).unwrap();
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .unwrap();
        let slime = build(&mut world).unwrap();
        let mut points = Vec::new();
        let span = |points: &[Vec3]| {
            let (low, high) = points
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p.y), h.max(p.y)));
            (low, high)
        };
        let mut highest = f32::MIN;
        for tick in 0..HOP + 30 {
            slime.drive(&mut world, tick);
            world.step(1.0 / 60.0, 1).unwrap();
            slime.points(&world, &mut points);
            if tick == HOP - 2 {
                let (low, high) = span(&points);
                assert!(low > -0.02 && high < 1.7 * RADIUS, "sits {low} to {high}");
            }
            if tick >= HOP {
                highest = highest.max(span(&points).0);
            }
        }
        assert!(highest > 0.2, "its bottom rose to {highest} m");
    }
}

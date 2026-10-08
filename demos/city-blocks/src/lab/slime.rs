//! The slimes in `physics-lab --lab creatures` (#179, #180): the tropical island's slime (its
//! `meshgen::slime`, the owner's pick), a squat drop of mint jelly with a darker nucleus floating
//! inside and two tall glossy eyes, as Jolt soft bodies hopping before the dogs, one in each of
//! the island's four flavours (mint, blue, pink, yellow; [`FLAVOURS`]).
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
/// The island's four flavours (`slime_tint` in its `slime.wgsl`): each slime's prop name (its
/// rows in the material table: the jelly, its eyes, its nucleus) and its tint.
pub(crate) const FLAVOURS: [(&str, [f32; 3]); 4] = [
    ("lab-slime-mint", [0.25, 0.95, 0.55]),
    ("lab-slime-blue", [0.30, 0.70, 1.00]),
    ("lab-slime-pink", [1.00, 0.55, 0.80]),
    ("lab-slime-yellow", [1.00, 0.85, 0.30]),
];
/// Where each slime's home is (x, z; it starts there, its bottom 2 cm up): two before the dogs,
/// two between them; and how many ticks after the first slime's its gait starts, so they bounce
/// out of step.
const STARTS: [(f64, f64, u64); 4] = [
    (-0.6, 2.35, 0),
    (0.6, 2.35, 23),
    (-0.5, 1.15, 45),
    (0.5, 1.15, 68),
];
/// The island's gait (`ti-sim`: a slime's phase runs at 4.2 rad/s): a bounce every `CYCLE`
/// ticks (1.5 s). Each starts with a hop, airborne for about half the cycle: it rises `HOP_UP`
/// m/s under `GRAVITY` of the world's (a floaty bounce, as the island's arc of half a cycle;
/// 0.15 m up, 0.59 s in the air), and goes `HOP_ALONG` m/s its way, which it picks at random
/// each hop, back towards home when it has strayed `LEASH` metres. One cycle in `IDLE` it sits
/// and only breathes.
pub(super) const CYCLE: u64 = 90;
const GRAVITY: f32 = 0.35;
const HOP_UP: f32 = 1.0;
const HOP_ALONG: f32 = 0.55;
const LEASH: f32 = 0.35;
/// How near another slime pushes its way away from it, metres between their middles
/// (the island's keep their distance; soft bodies pressed together crumple).
const SPACE: f32 = 1.5;
const IDLE: u64 = 4;
/// The wobble running up the body: its sideways shift a metre of height (the island's).
const WOBBLE: f32 = 0.025;
/// What keeps it upright (`World::keep_soft_body_upright`): its top point (the octahedron's +y),
/// the turn back, rad/s a sine of tilt, and the share of its spin damped a tick.
const TOP: u32 = 2;
const UPRIGHT_SPRING: f32 = 6.0;
const UPRIGHT_DAMPING: f32 = 0.3;
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

/// A slime: its body, its home and its gait's delay (`STARTS`), its index (its draws of chance
/// follow it), and its way this hop and the last (unit x, z) and whether it sits this cycle.
#[derive(Clone, Debug)]
pub(super) struct Slime {
    pub body: BodyId,
    home: [f32; 2],
    delay: u64,
    index: u64,
    way: [f32; 2],
    last_way: [f32; 2],
    idle: bool,
}

/// How a slime is drawn at a moment ([`Slime::look`]): which way it faces (radians about +y,
/// 0 facing +z), how far it stretches up (the island's squash and stretch, its width going as
/// the inverse square root), and its wobble's clock (seconds, offset per slime).
#[derive(Clone, Copy, Debug)]
pub(super) struct Look {
    pub heading: f32,
    pub stretch: f32,
    pub clock: f32,
    /// The wobble's sideways shift a metre of height.
    pub wobble: f32,
}

#[cfg(test)]
impl Look {
    /// Facing +z, unstretched, still: the body as the soft body has it.
    pub const REST: Look = Look {
        heading: 0.0,
        stretch: 1.0,
        clock: 0.0,
        wobble: 0.0,
    };
}

/// SplitMix64's finaliser: a draw of chance from `x`.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A unit direction on the ground drawn from `seed`, without trigonometry (D-016): a point of
/// the square kept when it falls in the ring between 0.2 and 1 from its middle.
fn direction(seed: u64) -> [f32; 2] {
    let unit = |x: u64| (mix(x) >> 40) as f32 / (1u64 << 24) as f32;
    for k in 0..64 {
        let x = 2.0 * unit(seed ^ (2 * k)) - 1.0;
        let z = 2.0 * unit(seed ^ (2 * k + 1)) - 1.0;
        let r2 = x * x + z * z;
        if (0.04..=1.0).contains(&r2) {
            let r = r2.sqrt();
            return [x / r, z / r];
        }
    }
    [0.0, 1.0]
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

/// Adds the four slimes to `world`, in the order of [`FLAVOURS`], facing +z (the camera).
pub(super) fn build(world: &mut World) -> Result<Vec<Slime>> {
    let s = surface();
    STARTS
        .iter()
        .enumerate()
        .map(|(index, &(x, z, delay))| {
            let body = world.add_soft_body(&SoftBodyDesc {
                points: &s.points,
                faces: &s.faces,
                position: DVec3::new(x, f64::from(FLAT * HEIGHT) + 0.02, z),
                inverse_mass: POINTS as f32 / MASS,
                compliance: COMPLIANCE,
                bend_compliance: BEND_COMPLIANCE,
                pressure: PRESSURE,
                friction: 0.4,
                restitution: 0.0,
                iterations: 5,
                gravity_factor: GRAVITY,
                user_data: 0,
            })?;
            Ok(Slime {
                body,
                home: [x as f32, z as f32],
                delay,
                index: index as u64,
                way: [0.0, 1.0],
                last_way: [0.0, 1.0],
                idle: false,
            })
        })
        .collect()
}

/// Drives every slime before the step of tick `tick` ([`Slime::drive`]), each knowing where the
/// others are.
pub(super) fn drive_all(slimes: &mut [Slime], world: &mut World, tick: u64) {
    let bodies: Vec<BodyId> = slimes.iter().map(|s| s.body).collect();
    let mut at = Vec::new();
    world.transforms(&bodies, &mut at);
    let places: Vec<[f32; 2]> = at
        .iter()
        .map(|t| [t.position.x as f32, t.position.z as f32])
        .collect();
    for s in slimes {
        s.drive(world, tick, &places);
    }
}

impl Slime {
    /// Before the step of tick `tick`: kept upright (the island's slimes never roll; pushed
    /// along and caught by the ground as it lands, a soft body tumbles), and at the start of
    /// each cycle of its gait a hop its way (drawn at random, or back home when it has strayed),
    /// or, one cycle in [`IDLE`], a rest.
    fn drive(&mut self, world: &mut World, tick: u64, others: &[[f32; 2]]) {
        world.keep_soft_body_upright(self.body, TOP, UPRIGHT_SPRING, UPRIGHT_DAMPING);
        let Some(t) = tick.checked_sub(self.delay) else {
            return;
        };
        if !t.is_multiple_of(CYCLE) {
            return;
        }
        let hop = t / CYCLE;
        let seed = mix(self.index.wrapping_mul(0x1_0000_0001) ^ hop);
        self.last_way = self.way;
        self.idle = hop > 0 && seed.is_multiple_of(IDLE);
        if self.idle {
            return;
        }
        let mut at = Vec::new();
        world.transforms(&[self.body], &mut at);
        let p = at[0].position;
        let home = [self.home[0] - p.x as f32, self.home[1] - p.z as f32];
        let away = (home[0] * home[0] + home[1] * home[1]).sqrt();
        // Steered as the island's: a wish drawn at random, pulled home as it strays (fully at
        // the leash), and pushed from every other within `SPACE`, the harder the nearer.
        let wish = direction(seed >> 8);
        let pull = (away / LEASH).min(2.0) / away.max(1e-4);
        let mut steer = [
            0.5 * wish[0] + home[0] * pull,
            0.5 * wish[1] + home[1] * pull,
        ];
        for q in others {
            let d = [p.x as f32 - q[0], p.z as f32 - q[1]];
            let r = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if r > 1e-4 && r < SPACE {
                let push = 3.0 * (SPACE - r) / SPACE / r;
                steer[0] += d[0] * push;
                steer[1] += d[1] * push;
            }
        }
        let size = (steer[0] * steer[0] + steer[1] * steer[1]).sqrt();
        self.way = if size > 1e-4 {
            [steer[0] / size, steer[1] / size]
        } else {
            wish
        };
        let [x, z] = self.way;
        world.push_soft_body(self.body, Vec3::new(x * HOP_ALONG, HOP_UP, z * HOP_ALONG));
    }

    /// How it is drawn `ticks` ticks from the start (between two ticks): its heading turning
    /// from its last way to this one over the first sixth of the cycle, and the island's squash
    /// and stretch, 1 + 0.2 sin of the gait's phase (stretched in the air, squashed on landing;
    /// half as much while it sits).
    pub(super) fn look(&self, ticks: f64) -> Look {
        let t = (ticks - self.delay as f64).max(0.0);
        let cycle = (t / CYCLE as f64).fract() as f32;
        let phase = std::f32::consts::TAU * cycle;
        let angle = |w: [f32; 2]| w[0].atan2(w[1]);
        let (from, to) = (angle(self.last_way), angle(self.way));
        let mut turn = to - from;
        if turn > std::f32::consts::PI {
            turn -= std::f32::consts::TAU;
        } else if turn < -std::f32::consts::PI {
            turn += std::f32::consts::TAU;
        }
        let eased = {
            let s = (cycle * 6.0).min(1.0);
            s * s * (3.0 - 2.0 * s)
        };
        let swing = if self.idle { 0.08 } else { 0.2 };
        Look {
            heading: from + turn * eased,
            stretch: 1.0 + swing * phase.sin(),
            clock: ticks as f32 / 60.0 + 1.7 * self.index as f32,
            wobble: WOBBLE,
        }
    }

    /// Its points in the world after the last step into `out`.
    pub(super) fn points(&self, world: &World, out: &mut Vec<Vec3>) {
        let origin = world.soft_body_vertices(self.body, out);
        for p in out.iter_mut() {
            *p = (origin + p.as_dvec3()).as_vec3();
        }
    }
}

/// The matrices that bend the slime's mesh, one a point into `out`: each takes the point from
/// its place at rest to `points[k]` (about the slime's mover), turned as the surface's normal
/// there turned. Then the island's look (`look`, its vertex shader's): the body turned to its
/// heading about its middle, stretched up and thinned about its bottom, and wobbling, each
/// height shifted sideways by a wave running up the body (0.025 m a metre of height, at
/// 5 rad/s).
pub(super) fn skin(points: &[Vec3], look: Look, out: &mut Vec<Mat4>) {
    let s = surface();
    let bottom = points.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let turned = Quat::from_rotation_y(look.heading);
    let shape = Mat4::from_translation(Vec3::Y * bottom)
        * Mat4::from_quat(turned)
        * Mat4::from_scale(Vec3::new(
            1.0 / look.stretch.sqrt(),
            look.stretch,
            1.0 / look.stretch.sqrt(),
        ))
        * Mat4::from_translation(-Vec3::Y * bottom);
    let side = turned * Vec3::X;
    out.clear();
    out.extend(
        s.points
            .iter()
            .zip(points)
            .zip(s.normals.iter().zip(normals(&s.faces, points)))
            .map(|((&rest, &at), (&was, now))| {
                let turn = Quat::from_rotation_arc(was, now.normalize_or(was));
                let height = at.y - bottom;
                let wobble = (look.clock * 5.0 + height * 15.0).sin() * look.wobble * height;
                Mat4::from_translation(side * wobble)
                    * shape
                    * Mat4::from_rotation_translation(turn, at - turn * rest)
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
        skin(&s.points, Look::REST, &mut m);
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
    fn the_slimes_sit_squat_bounce_in_turn_and_stay_home() {
        let mut world = World::new(&WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(10.0, 0.5, 10.0), 0.05, 1000.0).unwrap();
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .unwrap();
        let mut slimes = build(&mut world).unwrap();
        assert_eq!(slimes.len(), FLAVOURS.len());
        let mut points = Vec::new();
        let span = |points: &[Vec3]| {
            points
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p.y), h.max(p.y)))
        };
        // Each one's bottom at its highest in its first cycle, and the tick it was.
        let mut highest = [(f32::MIN, 0); 4];
        let mut farthest = [0.0_f32; 4];
        let mut closest = f32::MAX;
        for tick in 0..600 {
            drive_all(&mut slimes, &mut world, tick);
            world.step(1.0 / 60.0, 1).unwrap();
            let mut middles = Vec::new();
            for (k, s) in slimes.iter().enumerate() {
                s.points(&world, &mut points);
                let (low, high) = span(&points);
                let first = STARTS[k].2;
                if tick == first {
                    assert!(low > -0.02 && high < 0.45, "slime {k} sits {low} to {high}");
                }
                if (first..first + CYCLE).contains(&tick) && low > highest[k].0 {
                    highest[k] = (low, tick);
                }
                let middle = points.iter().sum::<Vec3>() / points.len() as f32;
                let away = (middle.x - s.home[0]).hypot(middle.z - s.home[1]);
                farthest[k] = farthest[k].max(away);
                middles.push(middle);
                let look = s.look(tick as f64);
                assert!((0.79..=1.21).contains(&look.stretch));
            }
            for (i, a) in middles.iter().enumerate() {
                for b in &middles[i + 1..] {
                    closest = closest.min((a.x - b.x).hypot(a.z - b.z));
                }
            }
        }
        for k in 0..4 {
            let (low, tick) = highest[k];
            assert!(low > 0.08, "slime {k}'s bottom rose to {low} m");
            // At the top of its hop, a sixth of a cycle in: a floaty bounce.
            let top = STARTS[k].2 + CYCLE / 6;
            assert!(tick.abs_diff(top) < 10, "slime {k} highest at tick {tick}");
            assert!(
                farthest[k] < LEASH + 0.6,
                "slime {k} strayed {} m",
                farthest[k]
            );
        }
        eprintln!("SLIME highest {highest:?} farthest {farthest:?} closest {closest}");
        assert!(closest > 0.6, "two slimes came within {closest} m");
    }
}

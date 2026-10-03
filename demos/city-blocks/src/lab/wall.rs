//! `physics-lab --lab break` (issue #142, Phase 3's step 6): a brick wall held together by mortar
//! that breaks, and a wrecking ball on a chain from a gantry. Every brick is a body, held to the
//! bricks beside it, under it and over it, and the bottom course to the floor, by a fixed joint
//! (`World::join_fixed`); after each step the joints that carried more than the mortar holds are
//! broken (`bonds`, shared with the bridge). Jolt saves whether each joint holds with the world, so
//! a broken wall restores, replays and goes through `--net` like the rest. R lets the ball go.

use std::sync::{Arc, OnceLock};

use anyhow::Result;
use forge_geom::city::{Block, Imported, Lathe, PropKind, PropSpec};
use forge_geom::fracture::{Polyhedron, hull_points, voronoi};
use forge_geom::procedural::TriMesh;
use forge_physics::{BodyDesc, BodyId, JointId, Shape, Transform, Velocity, World};
use glam::{DVec3, Mat4, Quat, Vec3};

use super::bonds::{Bonds, Limits};

/// A brick's half sizes (21.5 × 6.5 × 10.25 cm), its density, kg/m³, and the wall: courses of
/// stretchers in running bond, the even ones this many bricks long and the odd ones one fewer,
/// set half a brick along.
pub(super) const BRICK_HALF: [f32; 3] = [0.1075, 0.0325, 0.051_25];
const BRICK_DENSITY: f32 = 1800.0;
const COURSES: u32 = 24;
const BRICKS: u32 = 14;
/// What the mortar holds before it breaks: a force of 2 500 N and a torque of 150 N·m; and how far
/// it gives: its bricks moved apart by 3 mm, or turned by about 1.5° (the sine of half the angle).
const MORTAR: Limits = Limits {
    force: 2500.0,
    torque: 150.0,
    stretch: 0.003,
    twist: 0.013,
};
/// The solver's velocity and position iterations over the wall: enough for 24 courses of
/// joints to stand rigid (the world's 10 and 2 let it sag and lean).
const MORTAR_STEPS: (u32, u32) = (30, 10);
/// The wrecking ball: its radius, metres, and density (steel), its chain's length and the
/// pivot it hangs from, over the wall's front; it starts held back by 60°.
pub(super) const BALL_RADIUS: f32 = 0.45;
const BALL_DENSITY: f32 = 7800.0;
pub(super) const CHAIN: f32 = 5.5;
const PIVOT: DVec3 = DVec3::new(0.0, 6.6, 1.0);
/// The gantry: its posts' half sizes and how far apart along x, its beam's half sizes.
const POST_HALF: [f32; 3] = [0.15, 3.45, 0.15];
const POSTS_X: f32 = 2.6;
const BEAM_HALF: [f32; 3] = [2.75, 0.15, 0.15];
/// A concrete column behind the wall, in the ball's way: its half sizes, where it stands, its
/// density, and the pieces it breaks into (cut ahead of time) when a blow changes its velocity
/// by more than `SHATTER` m/s in a step.
const COLUMN_HALF: DVec3 = DVec3::new(0.2, 1.2, 0.2);
const COLUMN_AT: DVec3 = DVec3::new(0.0, 1.2, -1.2);
const CONCRETE_DENSITY: f32 = 2400.0;
pub(super) const PIECES: usize = 14;
const SHATTER: f32 = 1.5;
/// Where the pieces wait, asleep, until the column breaks.
const PARKED: DVec3 = DVec3::new(0.0, -520.0, 0.0);

/// The column's pieces: the Voronoi cells of points spread up it (one a slice of its height,
/// placed in it by a hash), each about its centre of volume and drawn about it, cut once.
fn pieces() -> &'static [(DVec3, Arc<TriMesh>)] {
    static PIECES_: OnceLock<Vec<(DVec3, Arc<TriMesh>)>> = OnceLock::new();
    PIECES_.get_or_init(|| {
        let h = COLUMN_HALF;
        let seeds: Vec<DVec3> = (0..PIECES as u64)
            .map(|k| {
                let slice = (k as f64 + 0.2 + 0.6 * super::unit(9100 + 3 * k)) / PIECES as f64;
                DVec3::new(
                    (2.0 * super::unit(9101 + 3 * k) - 1.0) * 0.9 * h.x,
                    (2.0 * slice - 1.0) * h.y,
                    (2.0 * super::unit(9102 + 3 * k) - 1.0) * 0.9 * h.z,
                )
            })
            .collect();
        voronoi(&Polyhedron::cuboid(h), &seeds)
            .into_iter()
            .map(|cell| {
                let cell = cell.expect("a seed inside the column has a cell");
                let centre = cell.centroid();
                (centre, Arc::new(cell.mesh(centre)))
            })
            .collect()
    })
}

/// The scene's props: a brick, the gantry's post and beam, the ball (its origin at its
/// centre), its chain (along +y from its middle), the column whole and its pieces (their cut
/// faces their second section).
pub(super) fn props() -> Vec<PropSpec> {
    let profile = (0..=32)
        .map(|k| {
            let a = std::f32::consts::PI * k as f32 / 32.0;
            (BALL_RADIUS * a.sin(), -BALL_RADIUS * a.cos())
        })
        .collect();
    let block = |name: &str, half: [f32; 3], radius: f32| PropSpec {
        name: name.to_owned(),
        kind: PropKind::Block(Block {
            half,
            radius,
            segments: 4,
        }),
    };
    let mut props = vec![
        // Rounded by 6 mm: the joints read as mortar lines.
        block("lab-brick", BRICK_HALF, 0.006),
        block("lab-post", POST_HALF, 0.02),
        block("lab-beam", BEAM_HALF, 0.02),
        PropSpec {
            name: "lab-wrecking-ball".to_owned(),
            kind: PropKind::Lathe(Lathe {
                profile,
                around: 48,
                along: 33,
                flutes: 0,
                flute_depth: 0.0,
                flute_span: (0.0, 0.0),
            }),
        },
        block("lab-chain", [0.035, 0.5 * CHAIN, 0.035], 0.01),
        block("lab-column", COLUMN_HALF.as_vec3().to_array(), 0.0),
    ];
    props.extend(pieces().iter().enumerate().map(|(k, (_, mesh))| PropSpec {
        name: format!("lab-column-piece@{k}"),
        kind: PropKind::Imported(Imported {
            key: format!("lab column piece {k} of {PIECES}"),
            mesh: Arc::clone(mesh),
            normal_weight: None,
        }),
    }));
    props
}

/// What the site puts in the world.
pub(super) struct Site {
    /// The gantry, drawn: (prop, transform).
    pub statics: Vec<(usize, Mat4)>,
    pub bricks: Vec<BodyId>,
    pub ball: BodyId,
    pub wall: Wall,
}

/// The wall's mortar (between its bricks, and the bottom course to the floor), what holds the
/// ball back until it is let go, and the column with its pieces (each with its centre in the
/// column's frame).
#[derive(Clone, Debug)]
pub(super) struct Wall {
    pub mortar: Bonds,
    pub hold: JointId,
    pub ball: BodyId,
    pub column: BodyId,
    pub pieces: Vec<(BodyId, DVec3)>,
}

/// Builds the site into `world`: the gantry drawn with `post` and `beam`, the wall, the ball,
/// the column and its pieces parked out of sight.
pub(super) fn build(world: &mut World, post: usize, beam: usize) -> Result<Site> {
    let at = |x: f32, y: f32, z: f32| Mat4::from_translation(Vec3::new(x, y, z));
    let z = PIVOT.z as f32;
    let statics = vec![
        (post, at(-POSTS_X, POST_HALF[1], z)),
        (post, at(POSTS_X, POST_HALF[1], z)),
        (beam, at(0.0, PIVOT.y as f32 + BEAM_HALF[1], z)),
    ];
    let post_shape = Shape::cuboid(Vec3::from_array(POST_HALF), 0.02, 0.0)?;
    let beam_shape = Shape::cuboid(Vec3::from_array(BEAM_HALF), 0.02, 0.0)?;
    for (shape, (_, m)) in [&post_shape, &post_shape, &beam_shape]
        .into_iter()
        .zip(&statics)
    {
        world.add_body(&BodyDesc::fixed(shape, m.w_axis.truncate().as_dvec3()))?;
    }
    let shape = Shape::cuboid(Vec3::from_array(BRICK_HALF), 0.005, BRICK_DENSITY)?;
    let (length, height) = (
        2.0 * f64::from(BRICK_HALF[0]),
        2.0 * f64::from(BRICK_HALF[1]),
    );
    // The courses from the floor up, each brick on the ones under it: their indices into the
    // bricks and where each was laid.
    let (mut bricks, mut laid) = (Vec::new(), Vec::new());
    let mut courses: Vec<Vec<u32>> = Vec::new();
    for c in 0..COURSES {
        let odd = c % 2 == 1;
        let n = if odd { BRICKS - 1 } else { BRICKS };
        let y = height * (f64::from(c) + 0.5);
        let mut course = Vec::new();
        for i in 0..n {
            let x =
                (f64::from(i) - f64::from(BRICKS - 1) * 0.5 + if odd { 0.5 } else { 0.0 }) * length;
            let at = DVec3::new(x, y, 0.0);
            course.push(bricks.len() as u32);
            bricks.push(world.add_body(&BodyDesc {
                friction: 0.7,
                ..BodyDesc::dynamic(&shape, at)
            })?);
            laid.push(at);
        }
        courses.push(course);
    }
    // The mortar: each brick to the next along its course, the bottom course to the floor, and
    // each brick to the two it sits on (an odd course's brick j on bricks j and j + 1 of the
    // even course under it; an even course's brick i on bricks i − 1 and i of the odd one).
    let mut mortar = Bonds::new(bricks.clone(), laid, MORTAR);
    for (c, course) in courses.iter().enumerate() {
        for pair in course.windows(2) {
            mortar.join(world, Some(pair[0]), pair[1], MORTAR_STEPS);
        }
        if c == 0 {
            for &brick in course {
                mortar.join(world, None, brick, MORTAR_STEPS);
            }
            continue;
        }
        let under = &courses[c - 1];
        for (i, &brick) in course.iter().enumerate() {
            let on: [Option<usize>; 2] = if c % 2 == 1 {
                [Some(i), Some(i + 1)]
            } else {
                [i.checked_sub(1), (i < under.len()).then_some(i)]
            };
            for j in on.into_iter().flatten() {
                mortar.join(world, Some(under[j]), brick, MORTAR_STEPS);
            }
        }
    }
    // The ball, held back by 60° towards the front (a 3, 4, 5 triangle's would need a sine:
    // cos 60° and sin 60° are ½ and √3 ⁄ 2), on its chain, and held there until let go.
    let back = DVec3::new(0.0, -0.5, 0.75_f64.sqrt()) * f64::from(CHAIN);
    let ball_shape = Shape::sphere(BALL_RADIUS, BALL_DENSITY)?;
    let ball = world.add_body(&BodyDesc {
        friction: 0.4,
        allow_sleep: false,
        ..BodyDesc::dynamic(&ball_shape, PIVOT + back)
    })?;
    world.join_distance(None, ball, PIVOT, PIVOT + back, (0.0, CHAIN));
    let hold = world.join_fixed(None, ball, (0, 0));
    // The column standing whole, and its pieces asleep on a shelf far under the floor.
    let column_shape = Shape::cuboid(COLUMN_HALF.as_vec3(), 0.01, CONCRETE_DENSITY)?;
    let column = world.add_body(&BodyDesc {
        friction: 0.7,
        ..BodyDesc::dynamic(&column_shape, COLUMN_AT)
    })?;
    let shelf = Shape::cuboid(Vec3::new(60.0, 0.5, 4.0), 0.05, 0.0)?;
    world.add_body(&BodyDesc::fixed(&shelf, PARKED - DVec3::new(0.0, 1.5, 0.0)))?;
    let mut parts = Vec::new();
    for (k, (centre, mesh)) in pieces().iter().enumerate() {
        let shape = Shape::convex_hull(&hull_points(mesh), 0.01, CONCRETE_DENSITY)?;
        let body = world.add_body(&BodyDesc {
            friction: 0.7,
            asleep: true,
            ..BodyDesc::dynamic(&shape, PARKED + DVec3::new(2.0 * k as f64, 0.0, 0.0))
        })?;
        parts.push((body, *centre));
    }
    Ok(Site {
        statics,
        bricks,
        ball,
        wall: Wall {
            mortar,
            hold,
            ball,
            column,
            pieces: parts,
        },
    })
}

impl Wall {
    /// After a step of `dt`: breaks the mortar that carried more than it holds or gave more
    /// than it gives ([`Bonds::crack`]). How many joints broke.
    pub(super) fn crack(&self, world: &mut World, dt: f32) -> usize {
        self.mortar.crack(world, dt)
    }

    /// The column's velocity, for [`Wall::shatter`] after the step.
    pub(super) fn column_velocity(&self, world: &World) -> Velocity {
        let mut v = Vec::new();
        world.velocities(&[self.column], &mut v);
        v[0]
    }

    /// After a step of `dt` from the column moving at `before`: when a blow changed its
    /// velocity by more than gravity and `SHATTER` m/s, the column breaks: its pieces take its
    /// place and its motion (each the velocity of its point of the column), and the column goes
    /// to the shelf under the floor. Whether it broke.
    pub(super) fn shatter(&self, world: &mut World, before: Velocity, dt: f32) -> bool {
        let (mut t, mut v) = (Vec::new(), Vec::new());
        world.transforms(&[self.column], &mut t);
        world.velocities(&[self.column], &mut v);
        let (at, now) = (t[0], v[0]);
        let gravity = Vec3::new(0.0, -9.81, 0.0) * dt;
        if at.position.y < PARKED.y * 0.5
            || (now.linear - before.linear - gravity).length() <= SHATTER
        {
            return false;
        }
        for &(piece, centre) in &self.pieces {
            let offset = at.rotation.as_dquat() * centre;
            world.set_transform(
                piece,
                Transform {
                    position: at.position + offset,
                    rotation: at.rotation,
                },
            );
            world.set_velocity(
                piece,
                Velocity {
                    linear: now.linear + now.angular.cross(offset.as_vec3()),
                    angular: now.angular,
                },
            );
        }
        world.set_transform(
            self.column,
            Transform {
                position: PARKED + DVec3::new(-40.0, COLUMN_HALF.y - 1.0, 0.0),
                rotation: Quat::IDENTITY,
            },
        );
        world.set_velocity(
            self.column,
            Velocity {
                linear: Vec3::ZERO,
                angular: Vec3::ZERO,
            },
        );
        true
    }

    /// Lets the ball go.
    pub(super) fn release(&self, world: &mut World) {
        world.set_holding(&[self.hold], false);
    }

    /// Whether the ball is still held back.
    pub(super) fn held(&self, world: &World) -> bool {
        let mut holding = Vec::new();
        world.holding(&[self.hold], &mut holding);
        holding[0]
    }

    /// The mortar's joints still holding.
    pub(super) fn holding(&self, world: &World) -> usize {
        self.mortar.holding(world)
    }

    /// The chain for the movers, after the bodies: from the pivot to the ball (for the eye
    /// only: it is drawn straight, its length the chain's).
    pub(super) fn chain(&self, world: &World, out: &mut Vec<Transform>) {
        let mut t = Vec::new();
        world.transforms(&[self.ball], &mut t);
        let towards = (t[0].position - PIVOT).as_vec3().normalize_or(Vec3::NEG_Y);
        out.push(Transform {
            position: PIVOT + (towards * 0.5 * CHAIN).as_dvec3(),
            rotation: Quat::from_rotation_arc(Vec3::NEG_Y, towards),
        });
    }
}

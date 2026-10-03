//! `physics-lab --lab tank` (issue #156, D-044's first milestone): a dam break in a glass tank
//! on a table. The tank is 1.6 m long, 0.6 m tall and 0.6 m deep inside (the owner's ask of
//! 2026-10-03: larger, the same 0.4 m of water behind the gate, so more water); the water stands
//! behind a gate 60 cm from its left end. Space (or `--release N`) lifts the gate: the water runs
//! out along the floor, climbs the far wall, falls back and settles at the level its volume
//! gives over the whole floor, 15 cm.
//!
//! The water is the GPU liquid (`forge_render::liquid`): 590 000 particles on a 1.25 cm grid, the
//! owner's budget of answer 3 (~640 000, "bigger or finer") spent on the larger tank
//! (`--liquid-cell 0.01`: 1.15 million), drawn through the glass. It is visual and lab-only
//! (answer 2): the lab's world holds the table and the gate, and hands the liquid where the gate
//! stands each substep.
//!
//! `--lab tank-bench`: the same tank as a bench to tune the liquid by (the owner's ask of
//! 2026-10-03, after Sebastian Lague's fluid videos): no glass to see, no frame, no table, the
//! tank on a floor of 10 cm squares in four tints (a metre a line) to read distances and speeds
//! off, a plain background, the sun alone, and the water tinted so its depth and motion show.
//!
//! `--lab tank-hole`: the glass tank with the gate fixed and a round hole through it, 8 cm across
//! and low in its middle (the owner's ask: "the dam open a circular hole"), shut by a shutter on
//! its dry side. Space (or `--release N`) slides the shutter up: the water jets out through the
//! hole at about √(2gh) and the reservoir drains until both sides stand level.

use anyhow::Result;
use std::sync::Arc;

use forge_geom::city::{Block, Imported, PropKind, PropSpec};
use forge_geom::procedural::TriMesh;
use forge_physics::{BodyDesc, BodyId, Motion, Shape, Transform, Velocity, World};
use forge_render::{LiquidHole, LiquidStep, LiquidTank};
use glam::{Mat4, UVec3, Vec2, Vec3};

/// The tank's inside, metres, its grid's cell by default (`--liquid-cell`) and its glass's
/// thickness.
const INSIDE: Vec3 = Vec3::new(1.6, 0.6, 0.6);
pub(crate) const CELL: f32 = 0.0125;
const GLASS: f32 = 0.01;
/// The table's top over the floor, metres.
const TABLE_TOP: f32 = 0.8;
const TABLE_HALF: [f32; 3] = [1.05, 0.5 * TABLE_TOP, 0.5];
/// The bench's floor: four squares of this half side round the tank, each its own tint.
const BENCH_HALF: f32 = 2.0;
/// The gate's faces along x from the inside's corner, and the water behind it at the start, metres:
/// as far as the gate, across the tank.
const GATE_X: [f32; 2] = [0.60, 0.62];
const WATER_HEIGHT: f32 = 0.4;
/// The gate: half its size (a little taller than the tank, so it stands out of it); how fast it
/// lifts, m/s, and how high its bottom goes (clear of the water as it runs out).
const GATE_HALF: [f32; 3] = [0.01, 0.31, 0.5 * INSIDE.z];
const GATE_LIFT: f32 = 3.0;
const GATE_RISE: f32 = 0.45;
/// The hole through the fixed gate of `tank-hole`: its centre's height over the floor and depth
/// across the tank, and its radius, metres. The shutter over it on the dry side: half its size,
/// how fast it slides up, m/s, and how high its bottom goes.
const HOLE: [f32; 3] = [0.12, 0.5 * INSIDE.z, 0.04];
const SHUTTER_HALF: [f32; 3] = [0.005, 0.08, 0.08];
const SHUTTER_LIFT: f32 = 2.0;
const SHUTTER_RISE: f32 = 0.65;
/// The frame's bars round the glass: their half thickness.
const BAR: f32 = 0.012;
/// Substeps of the liquid a tick of the lab's (1/240 s each).
pub(crate) const SUBSTEPS: usize = 4;

/// The inside's corner in the world: the tank in the table's middle on its glass floor, or on the
/// bench's floor.
pub(crate) fn corner(bench: bool) -> Vec3 {
    let floor = if bench { 0.0 } else { TABLE_TOP + GLASS };
    Vec3::new(-0.5 * INSIDE.x, floor, -0.5 * INSIDE.z)
}

/// The scene's props: the table, the frame's bars along x, y and z, the gate, the bench's four
/// floors, the holed gate and its shutter.
pub(super) fn props() -> Vec<PropSpec> {
    let block = |name: &str, half: [f32; 3], radius: f32| PropSpec {
        name: name.to_owned(),
        kind: PropKind::Block(Block {
            half,
            radius,
            segments: 4,
        }),
    };
    let outer = INSIDE + 2.0 * GLASS;
    vec![
        block("lab-tank-table", TABLE_HALF, 0.01),
        block("lab-tank-bar-x", [0.5 * outer.x + BAR, BAR, BAR], 0.003),
        block(
            "lab-tank-bar-y",
            [BAR, 0.5 * (INSIDE.y + GLASS), BAR],
            0.003,
        ),
        block("lab-tank-bar-z", [BAR, BAR, 0.5 * outer.z + BAR], 0.003),
        block("lab-tank-gate", GATE_HALF, 0.003),
        block("lab-bench-blue", [BENCH_HALF, 0.05, BENCH_HALF], 0.002),
        block("lab-bench-violet", [BENCH_HALF, 0.05, BENCH_HALF], 0.002),
        block("lab-bench-sand", [BENCH_HALF, 0.05, BENCH_HALF], 0.002),
        block("lab-bench-green", [BENCH_HALF, 0.05, BENCH_HALF], 0.002),
        PropSpec {
            name: "lab-tank-wall".to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("lab tank wall, a hole of {} m at {} m", HOLE[2], HOLE[0]),
                mesh: Arc::new(holed_plate(
                    Vec3::from_array(GATE_HALF),
                    Vec2::new(HOLE[0] - GATE_HALF[1], 0.0),
                    HOLE[2],
                    48,
                )),
                normal_weight: None,
            }),
        },
        block("lab-tank-shutter", SHUTTER_HALF, 0.002),
    ]
}

/// A plate of half size `half` (x its thickness) with a round hole through it along x, centred at
/// `centre` (y, z from the plate's middle), of `radius`, `around` sides round: each face a ring of
/// quads from the hole out to the plate's edge along rays from the hole's centre (the plate's
/// corners among them), the hole's wall smooth, the edges flat.
fn holed_plate(half: Vec3, centre: Vec2, radius: f32, around: usize) -> TriMesh {
    // In the plate's face, u along z and v along y, from the hole's centre.
    let (lo, hi) = (
        Vec2::new(-half.z - centre.y, -half.y - centre.x),
        Vec2::new(half.z - centre.y, half.y - centre.x),
    );
    let mut angles: Vec<f32> = (0..around)
        .map(|k| k as f32 / around as f32 * std::f32::consts::TAU)
        .collect();
    for corner in [hi, Vec2::new(lo.x, hi.y), lo, Vec2::new(hi.x, lo.y)] {
        angles.push(corner.y.atan2(corner.x).rem_euclid(std::f32::consts::TAU));
    }
    angles.sort_by(f32::total_cmp);
    angles.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    let edge = |d: Vec2| {
        let reach = |x: f32, l: f32, h: f32| {
            if x > 0.0 {
                h / x
            } else if x < 0.0 {
                l / x
            } else {
                f32::MAX
            }
        };
        d * reach(d.x, lo.x, hi.x).min(reach(d.y, lo.y, hi.y))
    };
    let at = |x: f32, p: Vec2| Vec3::new(x, centre.x + p.y, centre.y + p.x);
    let mut mesh = TriMesh::default();
    let tri = |mesh: &mut TriMesh, p: [Vec3; 3], n: [Vec3; 3]| {
        // Counter-clockwise seen from where the normals point.
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let order = if face.dot(n[0] + n[1] + n[2]) >= 0.0 {
            [0, 1, 2]
        } else {
            [0, 2, 1]
        };
        for k in order {
            mesh.indices.push(mesh.positions.len() as u32);
            mesh.positions.push(p[k].to_array());
            mesh.normals.push(n[k].to_array());
        }
    };
    let count = angles.len();
    for i in 0..count {
        let (a, b) = (angles[i], angles[(i + 1) % count]);
        let (da, db) = (Vec2::from_angle(a), Vec2::from_angle(b));
        for x in [half.x, -half.x] {
            let n = Vec3::new(x.signum(), 0.0, 0.0);
            let (ia, ib) = (at(x, da * radius), at(x, db * radius));
            let (oa, ob) = (at(x, edge(da)), at(x, edge(db)));
            tri(&mut mesh, [ia, oa, ob], [n; 3]);
            tri(&mut mesh, [ia, ob, ib], [n; 3]);
        }
        // The hole's wall, facing its axis.
        let (na, nb) = (
            -at(0.0, da) + at(0.0, Vec2::ZERO),
            -at(0.0, db) + at(0.0, Vec2::ZERO),
        );
        let (fa, fb) = (at(half.x, da * radius), at(half.x, db * radius));
        let (ba, bb) = (at(-half.x, da * radius), at(-half.x, db * radius));
        tri(&mut mesh, [fa, ba, bb], [na, na, nb]);
        tri(&mut mesh, [fa, bb, fb], [na, nb, nb]);
    }
    // The plate's four edges.
    for (n, u, v) in [
        (Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::X, Vec3::Y),
    ] {
        let middle = n * half;
        let (du, dv) = (u * half, v * half);
        let p = [
            middle - du - dv,
            middle + du - dv,
            middle + du + dv,
            middle - du + dv,
        ];
        tri(&mut mesh, [p[0], p[1], p[2]], [n; 3]);
        tri(&mut mesh, [p[0], p[2], p[3]], [n; 3]);
    }
    mesh
}

/// The tank as the world holds it: the gate's body (the lifting gate) or the shutter's (over the
/// fixed gate's hole), and where the tank's inside starts.
#[derive(Clone, Debug)]
pub(super) struct Tank {
    gate: Option<BodyId>,
    shutter: Option<BodyId>,
    corner: Vec3,
}

/// What the scene puts in the world: its still props, the tank, and what moves (its prop and body).
pub(super) struct Built {
    pub statics: Vec<(usize, Mat4)>,
    pub tank: Tank,
    pub mover: (usize, BodyId),
}

/// The tank's liquid on a grid of `cell` metres: its inside, the water and the gate (with `hole`,
/// the fixed gate's hole), in its own frame (the inside's corner).
pub(crate) fn liquid(cell: f32, hole: bool) -> LiquidTank {
    LiquidTank {
        // Whole cells: the inside within half a cell of the glass (at 1.5 cm, 1.005 m long).
        size: (INSIDE / cell).round() * cell,
        cell,
        water: UVec3::new(
            (GATE_X[0] / cell + 1e-3).floor() as u32,
            (WATER_HEIGHT / cell).round() as u32,
            (INSIDE.z / cell).round() as u32,
        ),
        gate: Some(GATE_X),
        hole: hole.then_some(LiquidHole {
            centre: Vec2::new(HOLE[0], HOLE[1]),
            radius: HOLE[2],
        }),
        glass: GLASS,
    }
}

/// Where the gate's middle stands while it is shut.
fn gate_home(corner: Vec3) -> Vec3 {
    corner + Vec3::new(0.5 * (GATE_X[0] + GATE_X[1]), GATE_HALF[1], 0.5 * INSIDE.z)
}

/// Where the shutter's middle stands over the hole, against the gate's dry side.
fn shutter_home(corner: Vec3) -> Vec3 {
    corner + Vec3::new(GATE_X[1] + SHUTTER_HALF[0], HOLE[0], HOLE[1])
}

/// Builds the table, the frame and the gate, or on the bench the gate and the floors; with `hole`,
/// the gate fixed with its hole and the shutter over it (`first`: the table's prop, the others
/// after it as in [`props`]).
pub(super) fn build(world: &mut World, first: usize, bench: bool, hole: bool) -> Result<Built> {
    let mut statics = Vec::new();
    let corner = corner(bench);
    let kinematic = |world: &mut World, half: [f32; 3], at: Vec3| -> Result<BodyId> {
        let shape = Shape::cuboid(Vec3::from_array(half), 0.002, 0.0)?;
        Ok(world.add_body(&BodyDesc {
            motion: Motion::Kinematic,
            ..BodyDesc::dynamic(&shape, at.as_dvec3())
        })?)
    };
    let (tank, mover) = if hole {
        statics.push((first + 9, Mat4::from_translation(gate_home(corner))));
        let shutter = kinematic(world, SHUTTER_HALF, shutter_home(corner))?;
        let tank = Tank {
            gate: None,
            shutter: Some(shutter),
            corner,
        };
        (tank, (first + 10, shutter))
    } else {
        let gate = kinematic(world, GATE_HALF, gate_home(corner))?;
        let tank = Tank {
            gate: Some(gate),
            shutter: None,
            corner,
        };
        (tank, (first + 4, gate))
    };
    if bench {
        let floor = Shape::cuboid(Vec3::new(BENCH_HALF, 0.05, BENCH_HALF), 0.002, 0.0)?;
        for (k, (x, z)) in [(-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (1.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            let at = Vec3::new(x * BENCH_HALF, -0.05, z * BENCH_HALF);
            world.add_body(&BodyDesc::fixed(&floor, at.as_dvec3()))?;
            statics.push((first + 5 + k, Mat4::from_translation(at)));
        }
        return Ok(Built {
            statics,
            tank,
            mover,
        });
    }
    let table_at = Vec3::new(0.0, TABLE_HALF[1], 0.0);
    let table = Shape::cuboid(Vec3::from_array(TABLE_HALF), 0.01, 0.0)?;
    world.add_body(&BodyDesc::fixed(&table, table_at.as_dvec3()))?;
    statics.push((first, Mat4::from_translation(table_at)));
    // The frame: bars along the glass box's twelve edges.
    let lo = corner - GLASS;
    let hi = corner + INSIDE + Vec3::new(GLASS, 0.0, GLASS);
    let mid = 0.5 * (lo + hi);
    for y in [lo.y, hi.y] {
        for z in [lo.z, hi.z] {
            statics.push((first + 1, Mat4::from_translation(Vec3::new(mid.x, y, z))));
        }
        for x in [lo.x, hi.x] {
            statics.push((first + 3, Mat4::from_translation(Vec3::new(x, y, mid.z))));
        }
    }
    for x in [lo.x, hi.x] {
        for z in [lo.z, hi.z] {
            statics.push((first + 2, Mat4::from_translation(Vec3::new(x, mid.y, z))));
        }
    }
    Ok(Built {
        statics,
        tank,
        mover,
    })
}

impl Tank {
    /// How far a body has risen from where it started, metres.
    fn risen(&self, world: &World, body: Option<BodyId>, home: Vec3) -> f32 {
        body.map_or(0.0, |body| {
            let mut t = Vec::new();
            world.transforms(&[body], &mut t);
            (t[0].position.y - f64::from(home.y)) as f32
        })
    }

    /// The gate's bottom and the shutter's over the tank's floor, metres: what the liquid's
    /// substeps take (the gate's 0 while it is shut, or where there is none).
    pub(super) fn state(&self, world: &World) -> [f32; 2] {
        let shutter_rest = HOLE[0] - SHUTTER_HALF[1];
        [
            self.risen(world, self.gate, gate_home(self.corner)),
            shutter_rest + self.risen(world, self.shutter, shutter_home(self.corner)),
        ]
    }

    /// Lifts the gate, or slides the shutter up.
    pub(super) fn open(&self, world: &mut World) {
        for (body, home, lift) in [
            (self.gate, gate_home(self.corner), GATE_LIFT),
            (self.shutter, shutter_home(self.corner), SHUTTER_LIFT),
        ] {
            if let Some(body) = body
                && self.risen(world, Some(body), home) <= 0.0
            {
                world.set_velocity(
                    body,
                    Velocity {
                        linear: Vec3::new(0.0, lift, 0.0),
                        angular: Vec3::ZERO,
                    },
                );
            }
        }
    }

    /// After a step: the lifted gate and shutter stop at their tops.
    pub(super) fn tick(&self, world: &mut World) {
        let shutter_rise = SHUTTER_RISE - (HOLE[0] - SHUTTER_HALF[1]);
        for (body, home, rise) in [
            (self.gate, gate_home(self.corner), GATE_RISE),
            (self.shutter, shutter_home(self.corner), shutter_rise),
        ] {
            let Some(body) = body else { continue };
            if self.risen(world, Some(body), home) < rise {
                continue;
            }
            world.set_velocity(
                body,
                Velocity {
                    linear: Vec3::ZERO,
                    angular: Vec3::ZERO,
                },
            );
            world.set_transform(
                body,
                Transform {
                    position: (home + Vec3::new(0.0, rise, 0.0)).as_dvec3(),
                    rotation: glam::Quat::IDENTITY,
                },
            );
        }
    }
}

/// The liquid's substeps over a tick whose gate and shutter went from `from` to `to` (their
/// bottoms over the floor, metres, as [`Tank::state`] gives them).
pub(crate) fn steps(from: [f32; 2], to: [f32; 2]) -> [LiquidStep; SUBSTEPS] {
    let speed = (to[0] - from[0]) / super::TICK;
    std::array::from_fn(|s| {
        let share = (s as f32 + 1.0) / SUBSTEPS as f32;
        LiquidStep {
            gate_bottom: from[0] + (to[0] - from[0]) * share,
            gate_speed: speed,
            shutter: from[1] + (to[1] - from[1]) * share,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_water_fills_the_reservoir_behind_the_gate() {
        let tank = liquid(CELL, false);
        assert_eq!(tank.cells(), UVec3::new(128, 48, 48));
        assert_eq!(tank.particles(), 589_824);
        // The block reaches the gate's near face and stands 0.4 m.
        let water = tank.water;
        assert!((water.x as f32 * CELL - GATE_X[0]).abs() < 1e-6);
        assert!((water.y as f32 * CELL - 0.4).abs() < 1e-6);
        // Settled over the whole floor, the gate lifted out: 15 cm.
        let volume = (water.x * water.y * water.z) as f32 * CELL * CELL * CELL;
        let level = volume / (INSIDE.x * INSIDE.z);
        assert!((level - 0.15).abs() < 1e-4, "{level}");
        // At 1 cm, 1.15 million; at 1.5 cm, short of the gate (39 cells, 0.585 m).
        assert_eq!(liquid(0.01, false).particles(), 1_152_000);
        let coarse = liquid(0.015, false);
        assert_eq!(coarse.water, UVec3::new(40, 27, 40));
        assert!(coarse.water.x as f32 * 0.015 <= GATE_X[0] + 1e-6);
    }

    #[test]
    fn the_holed_gate_is_a_plate_less_its_hole() {
        let half = Vec3::from_array(GATE_HALF);
        let mesh = holed_plate(half, Vec2::new(HOLE[0] - GATE_HALF[1], 0.0), HOLE[2], 48);
        let mut front = 0.0;
        for t in mesh.indices.chunks(3) {
            let p = [t[0], t[1], t[2]].map(|i| Vec3::from_array(mesh.positions[i as usize]));
            let n = Vec3::from_array(mesh.normals[t[0] as usize]);
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            // Counter-clockwise seen from where it faces.
            assert!(face.dot(n) >= 0.0);
            if n.x > 0.5 {
                front += 0.5 * face.length();
            }
        }
        let expected = 4.0 * half.y * half.z - std::f32::consts::PI * HOLE[2] * HOLE[2];
        assert!(
            (front - expected).abs() < 0.01 * std::f32::consts::PI * HOLE[2] * HOLE[2],
            "{front} {expected}"
        );
    }

    #[test]
    fn a_ticks_substeps_carry_the_gate_up_evenly() {
        let s = steps([0.0, 0.04], [0.05, 0.08]);
        assert_eq!(s.len(), SUBSTEPS);
        assert!((s[0].gate_bottom - 0.0125).abs() < 1e-6);
        assert!((s[3].gate_bottom - 0.05).abs() < 1e-6);
        assert!((s[0].gate_speed - 3.0).abs() < 1e-3);
        assert!((s[1].shutter - 0.06).abs() < 1e-6);
    }
}

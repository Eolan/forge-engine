//! `physics-lab --lab tank` (issue #156, D-044's first milestone): a dam break in a glass tank
//! on a table. The tank is 1.6 m long, 0.6 m tall and 0.6 m deep inside (the owner's ask of
//! 2026-10-03: larger, the same 0.4 m of water behind the gate, so more water); the water stands
//! behind a gate 60 cm from its left end. Space (or `--release N`) lifts the gate: the water runs
//! out along the floor, climbs the far wall, falls back and settles at the level its volume
//! gives over the whole floor, 15 cm.
//!
//! The water is the GPU liquid (`forge_render::liquid`): 590 000 particles on a 1.25 cm grid, the
//! owner's budget of answer 3 (~640 000, "bigger or finer") spent on the larger tank
//! (`--liquid-cell 0.01`: 1.15 million), drawn through the glass. It is visual and lab-only (answer 2): the lab's
//! world holds the table and the gate, and hands the liquid where the gate stands each substep.
//!
//! `--lab tank-bench`: the same tank as a bench to tune the liquid by (the owner's ask of
//! 2026-10-03, after Sebastian Lague's fluid videos): no glass to see, no frame, no table, the
//! tank on a floor of 10 cm squares in four tints (a metre a line) to read distances and speeds
//! off, a plain background, the sun alone, and the water tinted so its depth and motion show.

use anyhow::Result;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, Motion, Shape, Transform, Velocity, World};
use forge_render::{LiquidStep, LiquidTank};
use glam::{Mat4, UVec3, Vec3};

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
/// floors.
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
    ]
}

/// The tank as the world holds it: the gate's body, and where the tank's inside starts.
#[derive(Clone, Debug)]
pub(super) struct Tank {
    pub gate: BodyId,
    corner: Vec3,
}

/// What the scene puts in the world.
pub(super) struct Built {
    pub statics: Vec<(usize, Mat4)>,
    pub tank: Tank,
}

/// The tank's liquid on a grid of `cell` metres: its inside, the water and the gate, in its own
/// frame (the inside's corner).
pub(crate) fn liquid(cell: f32) -> LiquidTank {
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
        hole: None,
        glass: GLASS,
    }
}

/// Where the gate's middle stands while it is shut.
fn gate_home(corner: Vec3) -> Vec3 {
    corner + Vec3::new(0.5 * (GATE_X[0] + GATE_X[1]), GATE_HALF[1], 0.5 * INSIDE.z)
}

/// Builds the table, the frame and the gate, or on the bench the gate and the floors (`first`:
/// the table's prop, the others after it as in [`props`]).
pub(super) fn build(world: &mut World, first: usize, bench: bool) -> Result<Built> {
    let mut statics = Vec::new();
    let corner = corner(bench);
    let gate_shape = Shape::cuboid(Vec3::from_array(GATE_HALF), 0.003, 0.0)?;
    let gate = world.add_body(&BodyDesc {
        motion: Motion::Kinematic,
        ..BodyDesc::dynamic(&gate_shape, gate_home(corner).as_dvec3())
    })?;
    let tank = Tank { gate, corner };
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
        return Ok(Built { statics, tank });
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
    Ok(Built { statics, tank })
}

impl Tank {
    /// The gate's bottom over the tank's floor, metres.
    pub(super) fn gate_bottom(&self, world: &World) -> f32 {
        let mut t = Vec::new();
        world.transforms(&[self.gate], &mut t);
        (t[0].position.y - f64::from(gate_home(self.corner).y)) as f32
    }

    /// Lifts the gate.
    pub(super) fn open(&self, world: &mut World) {
        if self.gate_bottom(world) > 0.0 {
            return;
        }
        world.set_velocity(
            self.gate,
            Velocity {
                linear: Vec3::new(0.0, GATE_LIFT, 0.0),
                angular: Vec3::ZERO,
            },
        );
    }

    /// After a step: the lifted gate stops at its top.
    pub(super) fn tick(&self, world: &mut World) {
        if self.gate_bottom(world) < GATE_RISE {
            return;
        }
        world.set_velocity(
            self.gate,
            Velocity {
                linear: Vec3::ZERO,
                angular: Vec3::ZERO,
            },
        );
        world.set_transform(
            self.gate,
            Transform {
                position: (gate_home(self.corner) + Vec3::new(0.0, GATE_RISE, 0.0)).as_dvec3(),
                rotation: glam::Quat::IDENTITY,
            },
        );
    }
}

/// The liquid's substeps over a tick whose gate went from `from` to `to` (its bottom, metres).
pub(crate) fn steps(from: f32, to: f32) -> [LiquidStep; SUBSTEPS] {
    let speed = (to - from) / super::TICK;
    std::array::from_fn(|s| LiquidStep {
        gate_bottom: from + (to - from) * (s as f32 + 1.0) / SUBSTEPS as f32,
        gate_speed: speed,
        plugged: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_water_fills_the_reservoir_behind_the_gate() {
        let tank = liquid(CELL);
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
        assert_eq!(liquid(0.01).particles(), 1_152_000);
        let coarse = liquid(0.015);
        assert_eq!(coarse.water, UVec3::new(40, 27, 40));
        assert!(coarse.water.x as f32 * 0.015 <= GATE_X[0] + 1e-6);
    }

    #[test]
    fn a_ticks_substeps_carry_the_gate_up_evenly() {
        let s = steps(0.0, 0.05);
        assert_eq!(s.len(), SUBSTEPS);
        assert!((s[0].gate_bottom - 0.0125).abs() < 1e-6);
        assert!((s[3].gate_bottom - 0.05).abs() < 1e-6);
        assert!((s[0].gate_speed - 3.0).abs() < 1e-3);
    }
}

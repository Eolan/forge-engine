//! `physics-lab --lab flood` (issue #144, Phase 3's step 8): a dam break. A walled basin 48 m
//! by 24 m on the lab's floor, a reservoir of water 2 m deep behind a gate at its upper end,
//! concrete blocks and a hut downstream; crates, barrels and logs afloat behind the gate and
//! lying on the dry floor beyond it. The water is D-009's authoritative column model
//! (`forge_physics::shallow`), a cell every 25 cm, its bed raised where the walls, the gate and
//! the blocks stand; what floats is pushed by it as by the sea (its surface and its flow), and
//! pushes it aside in turn (#151: its volume under the water raises the surface the water's
//! slopes see).
//! Space (or `--release N`) lifts the gate: the water runs out down the basin, round the
//! blocks, carrying what floats. It splashes where the columns fail (#162): spray off its front
//! running over the dry floor and where it runs into a wall or a block ([`splashes`]).

use anyhow::Result;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::buoyancy::Hull;
use forge_physics::shallow::{DRY, Pool};
use forge_physics::{BodyDesc, BodyId, Motion, Shape, Transform, Velocity, World};
use forge_render::SplashSource;
use glam::{DVec3, Mat4, Vec2, Vec3};

/// The pool: cells along x and z, metres apart, the first cell's centre.
const CELLS: [usize; 2] = [192, 96];
const SPACING: f32 = 0.25;
const ORIGIN: [f64; 2] = [-23.875, -11.875];
/// The reservoir's water over the floor, metres, and where its gate stands along x.
const RESERVOIR: f32 = 2.0;
pub(super) const GATE_X: f32 = -14.0;
/// The walls round the basin, the gate, a block and the hut: half sizes, metres.
const WALL_HEIGHT: f32 = 2.6;
const WALL_X_HALF: [f32; 3] = [24.5, 0.5 * WALL_HEIGHT, 0.25];
const WALL_Z_HALF: [f32; 3] = [0.25, 0.5 * WALL_HEIGHT, 12.0];
const GATE_HALF: [f32; 3] = [0.2, 0.5 * WALL_HEIGHT, 12.0];
const BLOCK_HALF: [f32; 3] = [0.5, 0.9, 0.5];
const HUT_HALF: [f32; 3] = [1.5, 1.4, 1.5];
/// The blocks' and the hut's places on the floor (x, z).
const BLOCKS: [[f32; 2]; 4] = [[-6.0, -4.0], [-5.0, 5.0], [2.0, 0.5], [9.0, -6.5]];
const HUT: [f32; 2] = [5.5, -5.0];
/// How fast the gate lifts, m/s, and how high it goes.
const GATE_LIFT: f32 = 4.0;
const GATE_TOP: f32 = 4.5;

/// The scene's props: the walls along x and along z, the gate, a block, the hut.
pub(super) fn props() -> Vec<PropSpec> {
    let block = |name: &str, half: [f32; 3]| PropSpec {
        name: name.to_owned(),
        kind: PropKind::Block(Block {
            half,
            radius: 0.03,
            segments: 8,
        }),
    };
    vec![
        block("lab-dam-wall-x", WALL_X_HALF),
        block("lab-dam-wall-z", WALL_Z_HALF),
        block("lab-gate", GATE_HALF),
        block("lab-dam-block", BLOCK_HALF),
        block("lab-hut", HUT_HALF),
    ]
}

/// The still props' indices (the gate, a mover, is drawn with the third of [`props`]).
pub(super) struct Props {
    pub wall_x: usize,
    pub wall_z: usize,
    pub block: usize,
    pub hut: usize,
}

/// The dam as the world holds it: the gate's body and the pool's cells under it.
#[derive(Clone, Debug)]
pub(super) struct Dam {
    pub gate: BodyId,
    cells: Vec<usize>,
}

/// What the scene puts in the world.
pub(super) struct Basin {
    pub statics: Vec<(usize, Mat4)>,
    pub gate: BodyId,
    pub pool: Pool,
    pub dam: Dam,
}

/// Builds the basin into `world` and its water: the walls, the blocks and the hut, still; the
/// gate, a kinematic body; the pool's bed raised under them all, the reservoir filled.
pub(super) fn build(world: &mut World, props: &Props) -> Result<Basin> {
    let mut statics = Vec::new();
    let mut solids: Vec<(Vec3, [f32; 3])> = Vec::new();
    let mut place = |world: &mut World, prop: usize, half: [f32; 3], at: Vec3| -> Result<()> {
        let shape = Shape::cuboid(Vec3::from_array(half), 0.03, 0.0)?;
        world.add_body(&BodyDesc::fixed(&shape, at.as_dvec3()))?;
        statics.push((prop, Mat4::from_translation(at)));
        solids.push((at, half));
        Ok(())
    };
    let y = 0.5 * WALL_HEIGHT;
    let (x_end, z_end) = (24.0 + WALL_Z_HALF[0], 12.0 + WALL_X_HALF[2]);
    place(world, props.wall_x, WALL_X_HALF, Vec3::new(0.0, y, -z_end))?;
    place(world, props.wall_x, WALL_X_HALF, Vec3::new(0.0, y, z_end))?;
    place(world, props.wall_z, WALL_Z_HALF, Vec3::new(-x_end, y, 0.0))?;
    place(world, props.wall_z, WALL_Z_HALF, Vec3::new(x_end, y, 0.0))?;
    for [x, z] in BLOCKS {
        place(
            world,
            props.block,
            BLOCK_HALF,
            Vec3::new(x, BLOCK_HALF[1], z),
        )?;
    }
    place(
        world,
        props.hut,
        HUT_HALF,
        Vec3::new(HUT[0], HUT_HALF[1], HUT[1]),
    )?;
    let gate_at = Vec3::new(GATE_X, y, 0.0);
    let gate_shape = Shape::cuboid(Vec3::from_array(GATE_HALF), 0.03, 0.0)?;
    let gate = world.add_body(&BodyDesc {
        motion: Motion::Kinematic,
        ..BodyDesc::dynamic(&gate_shape, gate_at.as_dvec3())
    })?;
    // The pool: its bed the top of what stands on each cell's centre, the reservoir's water
    // behind the gate.
    let mut pool = Pool::new(CELLS, SPACING, ORIGIN);
    let covers = |at: Vec3, half: [f32; 3], x: f64, z: f64| {
        (x - f64::from(at.x)).abs() <= f64::from(half[0])
            && (z - f64::from(at.z)).abs() <= f64::from(half[2])
    };
    let mut cells = Vec::new();
    for cz in 0..CELLS[1] {
        for cx in 0..CELLS[0] {
            let (x, z) = (
                ORIGIN[0] + cx as f64 * f64::from(SPACING),
                ORIGIN[1] + cz as f64 * f64::from(SPACING),
            );
            let i = pool.index(cx, cz);
            for &(at, half) in &solids {
                if covers(at, half, x, z) {
                    pool.bed[i] = pool.bed[i].max(at.y + half[1]);
                }
            }
            if covers(gate_at, GATE_HALF, x, z) {
                pool.bed[i] = WALL_HEIGHT;
                cells.push(i);
            } else if x < f64::from(GATE_X) && pool.bed[i] < RESERVOIR {
                pool.depth[i] = RESERVOIR - pool.bed[i];
            }
        }
    }
    Ok(Basin {
        statics,
        gate,
        pool,
        dam: Dam { gate, cells },
    })
}

impl Dam {
    /// Whether the gate still holds the water back.
    pub(super) fn closed(&self, pool: &Pool) -> bool {
        self.cells.first().is_some_and(|&i| pool.bed[i] > 0.0)
    }

    /// Lifts the gate: its cells' bed down to the floor, the gate rising out of the water.
    pub(super) fn open(&self, world: &mut World, pool: &mut Pool) {
        if !self.closed(pool) {
            return;
        }
        for &i in &self.cells {
            pool.bed[i] = 0.0;
        }
        world.set_velocity(
            self.gate,
            Velocity {
                linear: Vec3::new(0.0, GATE_LIFT, 0.0),
                angular: Vec3::ZERO,
            },
        );
    }

    /// After a step: the lifted gate stops at the top.
    pub(super) fn tick(&self, world: &mut World) {
        let mut t = Vec::new();
        world.transforms(&[self.gate], &mut t);
        if t[0].position.y >= f64::from(GATE_TOP + 0.5 * WALL_HEIGHT) {
            let mut v = Vec::new();
            world.velocities(&[self.gate], &mut v);
            if v[0].linear.y > 0.0 {
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
                        position: DVec3::new(
                            f64::from(GATE_X),
                            f64::from(GATE_TOP + 0.5 * WALL_HEIGHT),
                            0.0,
                        ),
                        ..t[0]
                    },
                );
            }
        }
    }
}

/// The pool's samples for the water's drawing: the surface, the depth (0 where it is dry),
/// the velocity. A dry sample stands at its bed, so the water's edge thins onto the floor; one
/// whose bed stands over the water beside it (on a block, the hut, a wall, the gate) stands at
/// the highest wet neighbour's surface instead, so the water runs on level into what stands in
/// it, hidden there, rather than climbing its side as a sheet (the owner's report, 2026-10-03).
pub(super) fn samples(pool: &Pool) -> Vec<[f32; 4]> {
    let [nx, nz] = pool.size;
    let mut out = Vec::with_capacity(nx * nz);
    for z in 0..nz {
        for x in 0..nx {
            let i = pool.index(x, z);
            let d = pool.depth[i];
            let [u, w] = pool.velocity_at(x, z);
            let mut height = pool.surface(i);
            if d < DRY {
                let mut beside = None::<f32>;
                for bz in z.saturating_sub(1)..=(z + 1).min(nz - 1) {
                    for bx in x.saturating_sub(1)..=(x + 1).min(nx - 1) {
                        let j = pool.index(bx, bz);
                        if pool.depth[j] >= DRY {
                            let s = pool.surface(j);
                            beside = Some(beside.map_or(s, |b| b.max(s)));
                        }
                    }
                }
                if let Some(b) = beside {
                    height = height.min(b);
                }
            }
            out.push([height, if d < DRY { 0.0 } else { d }, u, w]);
        }
    }
    out
}

/// Where the floaters start: crates and barrels afloat behind the gate, logs and more crates
/// lying on the dry floor beyond it, a point each from `k`.
pub(super) fn afloat_at(k: u64, unit: impl Fn(u64) -> f64) -> DVec3 {
    let behind = k % 3 != 2;
    let (x0, x1) = if behind {
        (-22.5, -15.5)
    } else {
        (-12.0, -8.0)
    };
    let x = x0 + (x1 - x0) * unit(3 * k);
    let z = -10.0 + 20.0 * unit(3 * k + 1);
    let y = if behind {
        f64::from(RESERVOIR) + 0.3
    } else {
        0.6
    };
    DVec3::new(x, y + 0.3 * unit(3 * k + 2), z)
}

/// The water pushed aside by what floats (#151), from each floater's `volumes` under it: at its
/// centre of mass, over a footprint of three quarters of its hull's reach from the hull's middle
/// (a crate's 0.45 m, about its side; a log's spreads wider than the log).
pub(super) fn displace(
    pool: &mut Pool,
    world: &World,
    floaters: &[super::sea::Floater],
    hulls: &[Hull],
    volumes: &[f32],
) {
    let bodies: Vec<BodyId> = floaters.iter().map(|f| f.body).collect();
    let mut centers = Vec::new();
    world.centers_of_mass(&bodies, &mut centers);
    let items: Vec<([f64; 2], f32, f32)> = floaters
        .iter()
        .zip(&centers)
        .zip(volumes)
        .map(|((f, c), &volume)| {
            let vertices = hulls[f.hull].vertices();
            let middle = vertices.iter().copied().sum::<Vec3>() / vertices.len() as f32;
            let reach = vertices
                .iter()
                .fold(0.0_f32, |m, &v| m.max(v.distance(middle)));
            ([c.x, c.z], volume, 0.75 * reach)
        })
        .collect();
    pool.displace(&items);
}
/// The slowest water that splashes, m/s.
const SPLASH_SPEED: f32 = 1.5;

/// Where the flood's water splashes as it stands (#162's step 2): its front running over the dry
/// floor, and its water running into a wall, the gate's foot, a block or the hut. Each such
/// column is a bow pushing through the water (`SplashSource::Bow`, a column wide and long): the
/// front moving with the water, an obstacle moving against it, so the spray goes up and back.
/// From the authoritative columns, in their order, each column's seed its own: a replay splashes
/// alike.
pub(super) fn splashes(pool: &Pool, out: &mut Vec<SplashSource>) {
    let [nx, nz] = pool.size;
    let h = pool.spacing;
    for z in 0..nz {
        for x in 0..nx {
            let i = pool.index(x, z);
            if pool.depth[i] < DRY {
                continue;
            }
            let [u, w] = pool.velocity_at(x, z);
            let v = Vec2::new(u, w);
            if v.length() < SPLASH_SPEED {
                continue;
            }
            // The column ahead, along the flow's larger component.
            let (dx, dz) = if u.abs() >= w.abs() {
                (u.signum() as i64, 0)
            } else {
                (0, w.signum() as i64)
            };
            let (ax, az) = (x as i64 + dx, z as i64 + dz);
            let surface = pool.surface(i);
            let obstacle = if ax < 0 || az < 0 || ax >= nx as i64 || az >= nz as i64 {
                true
            } else {
                let j = pool.index(ax as usize, az as usize);
                if pool.bed[j] > surface {
                    true
                } else if pool.depth[j] < DRY {
                    false
                } else {
                    continue;
                }
            };
            // On the face between the two columns.
            let bow = Vec3::new(
                (pool.origin[0] + x as f64 * f64::from(h)) as f32 + 0.5 * h * dx as f32,
                surface,
                (pool.origin[1] + z as f64 * f64::from(h)) as f32 + 0.5 * h * dz as f32,
            );
            out.push(SplashSource::Bow {
                bow,
                velocity: if obstacle { -v } else { v },
                beam: h,
                length: h,
                seed: (i as u32 * 2 + u32::from(obstacle)) ^ 0x5eed_f100,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A channel 8 m long, water 1.5 m deep in its first half.
    fn channel(filled: usize) -> Pool {
        let mut pool = Pool::new([32, 4], 0.25, [0.0, 0.0]);
        for z in 0..4 {
            for x in 0..filled {
                let i = pool.index(x, z);
                pool.depth[i] = 1.5;
            }
        }
        pool
    }

    #[test]
    fn still_water_does_not_splash() {
        let pool = channel(32);
        let mut out = Vec::new();
        splashes(&pool, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_dam_break_sprays_off_its_front_then_off_the_wall() {
        let mut pool = channel(16);
        let mut front = false;
        let mut wall = false;
        for _ in 0..240 {
            pool.step(1.0 / 60.0);
            let mut out = Vec::new();
            splashes(&pool, &mut out);
            for s in out {
                let SplashSource::Bow { bow, velocity, .. } = s else {
                    panic!("only bows");
                };
                // The front runs down the channel; the water striking the far wall (x = 8 m)
                // sprays back up it.
                if velocity.x > 0.0 {
                    front = true;
                } else if bow.x > 7.5 {
                    wall = true;
                }
            }
        }
        assert!(front && wall, "front {front}, wall {wall}");
    }
}

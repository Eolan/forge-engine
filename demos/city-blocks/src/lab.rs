//! `physics-lab` (issue #136, Phase 3's first step, `docs/demos/physics-lab.md`): small scenes of
//! rigid bodies on a flat floor, each a test of `forge-physics` with its numbers and its
//! determinism hash, drawn by this demo's renderer as movers (#79).
//!
//! `--lab drop`: a stepped pyramid of 204 sandstone blocks, and 260 barrels, rocks and balls
//! dropped onto it and around it from 8 to 45 m. Space throws a ball from the camera; Enter
//! takes the scene back to its start (the world's saved state).
//!
//! The physics ticks at 60 Hz whatever the frame rate, the movers drawn between the last two
//! ticks; with `--fixed-step` a frame is a tick, so a capture at frame N shows tick N.

use std::time::Instant;

use anyhow::{Context as _, Result};
use forge_app::Context;
use forge_geom::city::{Block, Lathe, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, Velocity, World, WorldDesc, state_hash};
use forge_render::meshlet::MeshId;
use forge_render::{MeshletScene, MeshletSceneBuilder, MoverTransform};
use glam::{DVec3, Mat4, Quat, Vec3};

use super::{Args, CityMaterials, Cooked, barrel_prop, scene_origin};

/// The lab's scenes (`--lab`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum LabScene {
    /// A pyramid of blocks under a rain of barrels, rocks and balls.
    Drop,
}

/// Seconds a tick.
const TICK: f32 = 1.0 / 60.0;
/// The floor's half side, metres.
const FLOOR_HALF: f32 = 400.0;
/// The pyramid's blocks: half their side, and the layers (the bottom one this many a side).
const BLOCK_HALF: f32 = 0.4;
const PYRAMID_LAYERS: u32 = 8;
/// The balls' radius, metres.
const BALL_RADIUS: f32 = 0.25;
/// The rocks: their radii, metres (the city's boulders made small).
const ROCK_RADII: [f32; 3] = [0.35, 0.5, 0.65];
/// What rains on the pyramid: barrels, rocks (of each size in turn), balls.
const RAIN_BARRELS: u32 = 100;
const RAIN_ROCKS: u32 = 100;
const RAIN_BALLS: u32 = 60;
/// The balls Space throws, taken in turn, and how fast they leave the camera, m/s.
const THROWN: u32 = 32;
const THROW_SPEED: f32 = 25.0;
/// Where the balls not yet thrown wait, asleep, out of sight.
const PARKED_Y: f64 = -500.0;

/// The props the lab draws, in this order: the floor, the block, the barrel, the rocks, the
/// ball.
pub(crate) fn props() -> Vec<PropSpec> {
    let mut props = vec![
        PropSpec {
            name: "lab-floor".to_owned(),
            kind: PropKind::Block(Block {
                half: [FLOOR_HALF, 0.5, FLOOR_HALF],
                radius: 0.05,
                segments: 64,
            }),
        },
        PropSpec {
            name: "lab-block".to_owned(),
            kind: PropKind::Block(Block {
                half: [BLOCK_HALF; 3],
                radius: 0.03,
                segments: 4,
            }),
        },
        PropSpec {
            name: "lab-barrel".to_owned(),
            ..barrel_prop()
        },
    ];
    for (i, &radius) in ROCK_RADII.iter().enumerate() {
        props.push(PropSpec {
            name: format!("lab-rock-{}", i + 1),
            kind: PropKind::Boulder {
                seed: 1360 + i as u64,
                radius,
                segments: 24,
            },
        });
    }
    // A ball turned on the lathe: a half circle from its bottom to its top.
    let profile = (0..=32)
        .map(|k| {
            let a = std::f32::consts::PI * k as f32 / 32.0;
            (BALL_RADIUS * a.sin(), BALL_RADIUS * (1.0 - a.cos()))
        })
        .collect();
    props.push(PropSpec {
        name: "lab-ball".to_owned(),
        kind: PropKind::Lathe(Lathe {
            profile,
            around: 48,
            along: 33,
            flutes: 0,
            flute_depth: 0.0,
            flute_span: (0.0, 0.0),
        }),
    });
    props
}

/// A number in [0, 1) from `x`, exactly (SplitMix64's finaliser).
fn unit(x: u64) -> f64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

/// A rotation from `seed` made without trigonometry: four numbers normalised (the platforms'
/// `sin` differ in their last bits, a square root does not).
fn rotation(seed: u64) -> Quat {
    let c = |k: u64| (2.0 * unit(seed * 4 + k) - 1.0) as f32;
    Quat::from_xyzw(c(0), c(1), c(2), c(3)).normalize()
}

/// A group of bodies drawn with one mesh, in the movers' order.
struct Group {
    mesh: MeshId,
    bodies: Vec<BodyId>,
}

/// The lab's running state: the world, its bodies by mesh, the transforms of the last two ticks.
pub(crate) struct Lab {
    world: World,
    /// Every body in the movers' order (the groups one after the other).
    bodies: Vec<BodyId>,
    /// The balls Space throws: their bodies, and the next to throw.
    thrown: Vec<BodyId>,
    next_throw: usize,
    /// The state at tick 0, which Enter goes back to.
    start: Vec<u8>,
    previous: Vec<Transform>,
    current: Vec<Transform>,
    /// Seconds not yet ticked.
    pending: f32,
    /// Ticks since the start (or the last reset).
    pub tick: u64,
    /// Milliseconds the ticks took since the last title, and over the run.
    tick_ms: Vec<f64>,
    run_tick_ms: Vec<f64>,
    /// Bodies awake at the last tick.
    awake: u32,
    /// Ticks at which the hash is logged.
    logged: [u64; 3],
}

/// Builds the lab's scene and its world: the floor and the shapes, the bodies of `kind`, the
/// movers that draw them.
pub(crate) fn build(
    ctx: &Context,
    args: &Args,
    cooked: Cooked,
    kind: LabScene,
) -> Result<(MeshletScene, Lab)> {
    let start = Instant::now();
    let props = props();
    let mut builder = MeshletSceneBuilder::new();
    let ids: Vec<MeshId> = cooked.meshes.iter().map(|m| builder.add_mesh(m)).collect();
    CityMaterials::new(&ctx.device)?.apply(&mut builder, &props, &ids);
    builder.set_ray_traced(!args.no_shadows);
    builder.set_origin(scene_origin(args));
    let (floor, block, barrel, rocks, ball) = (ids[0], ids[1], ids[2], &ids[3..6], ids[6]);
    builder.add_instance(floor, Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0)));

    let mut world = World::new(&WorldDesc {
        // The client's rule (D-005): the cores less two, the caller one of them.
        threads: (std::thread::available_parallelism().map_or(4, |n| n.get()) / 2)
            .saturating_sub(3)
            .max(1) as u32,
        ..WorldDesc::default()
    });
    let floor_shape = Shape::cuboid(Vec3::new(FLOOR_HALF, 0.5, FLOOR_HALF), 0.05, 0.0)?;
    world.add_body(&BodyDesc::fixed(&floor_shape, DVec3::new(0.0, -0.5, 0.0)))?;
    // The shapes, each with its origin where its mesh has its own: the barrel's and the ball's
    // at their bottom, the rocks' hulls from their meshes' vertices.
    let block_shape = Shape::cuboid(Vec3::splat(BLOCK_HALF), 0.03, 2300.0)?;
    let up = |h: f32| (Vec3::new(0.0, h, 0.0), Quat::IDENTITY);
    let half_length = super::BARREL_LENGTH * 0.5;
    let (at, turn) = up(half_length);
    let barrel_shape =
        Shape::cylinder(half_length, super::BARREL_RADIUS, 0.03, 300.0)?.offset(at, turn)?;
    let (at, turn) = up(BALL_RADIUS);
    let ball_shape = Shape::sphere(BALL_RADIUS, 500.0)?.offset(at, turn)?;
    let rock_shapes = props[3..6]
        .iter()
        .map(|spec| {
            let mesh = spec.generate();
            let step = (mesh.positions.len() / 256).max(1);
            let points: Vec<Vec3> = mesh
                .positions
                .iter()
                .step_by(step)
                .map(|&p| Vec3::from(p))
                .collect();
            Shape::convex_hull(&points, 0.02, 2600.0)
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut groups = Vec::new();
    match kind {
        LabScene::Drop => {
            // The pyramid: layer L has (8 − L)² blocks, 2 cm apart, each a hair over the one
            // under it.
            let mut bodies = Vec::new();
            let pitch = 2.0 * BLOCK_HALF + 0.02;
            for layer in 0..PYRAMID_LAYERS {
                let side = PYRAMID_LAYERS - layer;
                for j in 0..side {
                    for i in 0..side {
                        let offset =
                            |k: u32| (f64::from(k) - f64::from(side - 1) * 0.5) * f64::from(pitch);
                        let position = DVec3::new(
                            offset(i),
                            f64::from(BLOCK_HALF)
                                + f64::from(layer) * f64::from(2.0 * BLOCK_HALF + 0.001),
                            offset(j),
                        );
                        bodies.push(world.add_body(&BodyDesc {
                            friction: 0.7,
                            ..BodyDesc::dynamic(&block_shape, position)
                        })?);
                    }
                }
            }
            groups.push(Group {
                mesh: block,
                bodies,
            });
            // The rain: barrels, rocks and balls over a disc of 10 m around the pyramid, from 8
            // to 45 m up, turned and spinning at random.
            let rain = |k: u64| {
                let (r, a, h) = (unit(3 * k), unit(3 * k + 1), unit(3 * k + 2));
                // An angle without trigonometry: a point of the unit square folded onto the
                // disc by its length (a square root only).
                let (x, z) = (2.0 * a - 1.0, 2.0 * unit(3 * k + 1_000_003) - 1.0);
                let len = (x * x + z * z).sqrt().max(1e-6);
                let radius = 10.0 * r.sqrt();
                DVec3::new(x / len * radius, 8.0 + 37.0 * h, z / len * radius)
            };
            let spin = |k: u64| {
                Vec3::new(
                    (2.0 * unit(k * 7 + 1) - 1.0) as f32,
                    (2.0 * unit(k * 7 + 2) - 1.0) as f32,
                    (2.0 * unit(k * 7 + 3) - 1.0) as f32,
                ) * 3.0
            };
            let mut k = 1_000u64;
            let mut barrels = Vec::new();
            for _ in 0..RAIN_BARRELS {
                k += 1;
                barrels.push(world.add_body(&BodyDesc {
                    rotation: rotation(k),
                    angular_velocity: spin(k),
                    friction: 0.5,
                    restitution: 0.2,
                    angular_damping: 0.3,
                    mass: Some(60.0),
                    ..BodyDesc::dynamic(&barrel_shape, rain(k))
                })?);
            }
            groups.push(Group {
                mesh: barrel,
                bodies: barrels,
            });
            let mut by_size: [Vec<BodyId>; 3] = Default::default();
            for n in 0..RAIN_ROCKS {
                k += 1;
                let size = n as usize % 3;
                by_size[size].push(world.add_body(&BodyDesc {
                    rotation: rotation(k),
                    angular_velocity: spin(k),
                    friction: 0.8,
                    restitution: 0.05,
                    ..BodyDesc::dynamic(&rock_shapes[size], rain(k))
                })?);
            }
            for (size, bodies) in by_size.into_iter().enumerate() {
                groups.push(Group {
                    mesh: rocks[size],
                    bodies,
                });
            }
            let mut balls = Vec::new();
            for _ in 0..RAIN_BALLS {
                k += 1;
                balls.push(world.add_body(&BodyDesc {
                    angular_velocity: spin(k),
                    friction: 0.6,
                    restitution: 0.6,
                    angular_damping: 0.3,
                    ..BodyDesc::dynamic(&ball_shape, rain(k))
                })?);
            }
            groups.push(Group {
                mesh: ball,
                bodies: balls,
            });
        }
    }
    // The balls to throw, asleep out of sight until thrown: the last group's last bodies.
    let mut thrown = Vec::new();
    for n in 0..THROWN {
        thrown.push(world.add_body(&BodyDesc {
            friction: 0.6,
            restitution: 0.6,
            angular_damping: 0.3,
            ccd: true,
            asleep: true,
            ..BodyDesc::dynamic(&ball_shape, DVec3::new(f64::from(n) * 2.0, PARKED_Y, 0.0))
        })?);
    }
    groups
        .last_mut()
        .context("the lab's last group is its balls")?
        .bodies
        .extend(&thrown);
    world.optimize_broad_phase();

    let per_mesh: Vec<(MeshId, u32)> = groups
        .iter()
        .map(|g| (g.mesh, g.bodies.len() as u32))
        .collect();
    builder.reserve_movers(&per_mesh);
    let mut scene = builder.build(&ctx.device)?;
    scene.build_tlas(&ctx.device, &ctx.shaders)?;

    let bodies: Vec<BodyId> = groups
        .iter()
        .flat_map(|g| g.bodies.iter().copied())
        .collect();
    let mut current = Vec::new();
    world.transforms(&bodies, &mut current);
    let start_state = world.save_state();
    tracing::info!(
        scene = ?kind,
        bodies = bodies.len(),
        threads = world.threads(),
        state_kib = start_state.len() / 1024,
        ms = start.elapsed().as_millis(),
        "physics lab ready"
    );
    Ok((
        scene,
        Lab {
            world,
            bodies,
            thrown,
            next_throw: 0,
            start: start_state,
            previous: current.clone(),
            current,
            pending: 0.0,
            tick: 0,
            tick_ms: Vec::new(),
            run_tick_ms: Vec::new(),
            awake: 0,
            logged: [60, 300, 600],
        },
    ))
}

impl Lab {
    /// Runs the ticks `dt` seconds owe (one a frame with `fixed`), keeping the last two ticks'
    /// transforms.
    pub(crate) fn advance(&mut self, dt: f32, fixed: bool) {
        let ticks = if fixed {
            1
        } else {
            self.pending = (self.pending + dt).min(0.25);
            let n = (self.pending / TICK) as u32;
            self.pending -= n as f32 * TICK;
            n
        };
        for _ in 0..ticks {
            let start = Instant::now();
            if let Err(e) = self.world.step(TICK, 1) {
                tracing::warn!("physics tick {}: {e}", self.tick);
            }
            let ms = start.elapsed().as_secs_f64() * 1e3;
            self.tick_ms.push(ms);
            self.run_tick_ms.push(ms);
            self.tick += 1;
            std::mem::swap(&mut self.previous, &mut self.current);
            self.world.transforms(&self.bodies, &mut self.current);
            if self.logged.contains(&self.tick) {
                self.awake = self.world.active_bodies();
                tracing::info!(
                    tick = self.tick,
                    hash = format!("{:#018x}", state_hash(&self.current)),
                    awake = self.awake,
                    "physics lab state"
                );
            }
        }
        self.awake = self.world.active_bodies();
    }

    /// The movers' transforms, between the last two ticks by the time not yet ticked.
    pub(crate) fn movers(&self) -> Vec<MoverTransform> {
        let t = (self.pending / TICK).clamp(0.0, 1.0);
        self.previous
            .iter()
            .zip(&self.current)
            .map(|(a, b)| MoverTransform {
                position: a.position.lerp(b.position, f64::from(t)).as_vec3(),
                rotation: a.rotation.slerp(b.rotation, t),
                scale: 1.0,
            })
            .collect()
    }

    /// Throws the next ball from `from` along `forward`.
    pub(crate) fn throw(&mut self, from: Vec3, forward: Vec3) {
        let ball = self.thrown[self.next_throw];
        self.next_throw = (self.next_throw + 1) % self.thrown.len();
        let forward = forward.normalize_or(Vec3::NEG_Z);
        let at = (from + forward * 1.0 - Vec3::new(0.0, BALL_RADIUS, 0.0)).as_dvec3();
        self.world.set_transform(
            ball,
            Transform {
                position: at,
                rotation: Quat::IDENTITY,
            },
        );
        self.world.set_velocity(
            ball,
            Velocity {
                linear: forward * THROW_SPEED,
                angular: Vec3::ZERO,
            },
        );
        // Drawn where it starts, not swept from its parking place.
        if let Some(k) = self.bodies.iter().position(|&b| b == ball) {
            let t = Transform {
                position: at,
                rotation: Quat::IDENTITY,
            };
            self.previous[k] = t;
            self.current[k] = t;
        }
    }

    /// Back to tick 0.
    pub(crate) fn reset(&mut self) {
        if let Err(e) = self.world.restore_state(&self.start) {
            tracing::warn!("the lab's start state: {e}");
            return;
        }
        self.world.transforms(&self.bodies, &mut self.current);
        self.previous.clone_from(&self.current);
        self.tick = 0;
        self.next_throw = 0;
        self.pending = 0.0;
    }

    /// The title's part: the ticks' time since the last title, and the bodies awake.
    pub(crate) fn title(&mut self) -> String {
        let n = self.tick_ms.len().max(1) as f64;
        let mean = self.tick_ms.iter().sum::<f64>() / n;
        let max = self.tick_ms.iter().copied().fold(0.0, f64::max);
        self.tick_ms.clear();
        format!(
            "physics {mean:.2} ms a tick (max {max:.2}), {} of {} awake, tick {}",
            self.awake,
            self.bodies.len(),
            self.tick
        )
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let mut ticks = std::mem::take(&mut self.run_tick_ms);
        if ticks.is_empty() {
            return;
        }
        let mean = ticks.iter().sum::<f64>() / ticks.len() as f64;
        tracing::info!(
            ticks = ticks.len(),
            mean = format!("{mean:.3}"),
            p99 = format!("{:.3}", super::percentile(&mut ticks, 0.99)),
            max = format!("{:.3}", super::percentile(&mut ticks, 1.0)),
            hash = format!("{:#018x}", state_hash(&self.current)),
            awake = self.awake,
            "physics ticks (ms) over the run"
        );
    }
}

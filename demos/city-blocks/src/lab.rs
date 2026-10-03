//! `physics-lab` (issues #136 and #137, Phase 3's first steps, `docs/demos/physics-lab.md`):
//! small scenes of rigid bodies on a flat floor, each a test of `forge-physics` with its numbers
//! and its determinism hash, drawn by this demo's renderer as movers (#79).
//!
//! `--lab drop`: a stepped pyramid of 204 sandstone blocks, and 260 barrels, rocks and balls
//! dropped onto it and around it from 8 to 45 m. Space throws a ball from the camera; Enter
//! takes the scene back to its start.
//!
//! The world is a `forge_sim::Simulation` (#137): it ticks at 60 Hz whatever the frame rate, the
//! movers drawn between the last two ticks, and what the player does reaches it as commands
//! stamped with their tick. With `--fixed-step` a frame is a tick, so a capture at frame N shows
//! tick N. `--record FILE` writes the session's commands and digests at exit, `--replay FILE`
//! plays them again and checks the digests; `--net MS` runs the scene through a server and this
//! player's client over a link of MS one way, with a second player (a bot) throwing too.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use forge_app::Context;
use forge_geom::city::{Block, Lathe, PropKind, PropSpec};
use forge_physics::buoyancy::{Fluid, Hull};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, Velocity, World, WorldDesc};
use forge_render::meshlet::MeshId;
use forge_render::{MeshletScene, MeshletSceneBuilder, MoverTransform};
use forge_sim::{
    Client, Codec, InputPacket, Link, LinkParams, PlayerId, Recording, Server, Simulation,
    Snapshot, Stamped, TICK, TICK_RATE,
};
use forge_task::TaskPool;
use glam::{DVec3, Mat4, Quat, Vec2, Vec3};

use super::{Args, CityMaterials, Cooked, barrel_prop, scene_origin};

mod creatures;
mod dominoes;
mod drive;
mod flood;
mod fly;
mod sea;
mod walk;
mod wall;

pub(crate) use walk::{RUN_SPEED, WALK_SPEED};

/// The lab's scenes (`--lab`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum LabScene {
    /// A pyramid of blocks under a rain of barrels, rocks and balls.
    Drop,
    /// The sea: crates, barrels, logs and balls afloat, rocks that sink, a jetty, a boat
    /// (#138).
    Sea,
    /// A playground to walk in: stairs, ramps, a moving platform, crates, blocks (#139).
    Walk,
    /// A car on a test track: a jump, a slalom, a wall of crates (#140).
    Drive,
    /// An aeroplane on a runway (#141).
    Fly,
    /// A brick wall held by mortar that breaks, and a wrecking ball (#142).
    Break,
    /// Creatures as powered ragdolls: mannequins on stands and dogs (#143).
    Creatures,
    /// A dam break: a reservoir behind a gate, a basin with blocks and a hut, what floats (#144).
    Flood,
    /// A domino run on a spiral (#146).
    Dominoes,
}

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
/// The balls the players throw, taken in turn, and how fast they leave, m/s.
const THROWN: u32 = 32;
const THROW_SPEED: f32 = 25.0;
/// Where the balls not yet thrown wait, asleep, out of sight.
const PARKED_Y: f64 = -500.0;
/// The props' indices in [`props`].
const FLOOR: usize = 0;
const BLOCK: usize = 1;
const BARREL: usize = 2;
const ROCKS: [usize; 3] = [3, 4, 5];
const BALL: usize = 6;
const CRATE: usize = 7;
const LOG: usize = 8;
const PILLAR: usize = 9;
const DECK: usize = 10;
const BOAT: usize = 11;
const SLAB: usize = 12;
const RAMP: usize = 13;
const PLATFORM: usize = 14;
const PLAYER: usize = 15;
const VISOR: usize = 16;
const CAR: usize = 17;
const WHEEL: usize = 18;
const PLANE: usize = 19;
const PROPELLER: usize = 20;
const RUNWAY: usize = 21;
const FIELD: usize = 22;
const BRICK: usize = 23;
const POST: usize = 24;
const BEAM: usize = 25;
const WRECKING_BALL: usize = 26;
const CHAIN: usize = 27;
const COLUMN: usize = 28;
/// The column's pieces, each its own prop from here.
const PIECE: usize = 29;
/// The creatures' parts, a kind's eleven after the other's, then the mannequins' pole.
const CREATURE: usize = PIECE + wall::PIECES;
const POLE: usize = CREATURE + 2 * creatures::PARTS;
/// The flood's walls along x and z, its gate, a block, the hut.
const FLOOD: usize = POLE + 1;
/// The dominoes' prop, after the flood's five.
const DOMINO: usize = FLOOD + 5;
/// What the sea scene sets afloat: crates, barrels, logs, balls, and rocks that sink.
const SEA_CRATES: u32 = 30;
const SEA_BARRELS: u32 = 30;
const SEA_LOGS: u32 = 16;
const SEA_BALLS: u32 = 20;
const SEA_ROCKS: u32 = 18;
/// What the flood carries: crates, barrels, logs.
const FLOOD_CRATES: u32 = 18;
const FLOOD_BARRELS: u32 = 12;
const FLOOD_LOGS: u32 = 9;
/// Fresh water: the sea's drag, its density 1000 kg/m³.
const FRESH: Fluid = Fluid {
    density: 1000.0,
    ..Fluid::SEA
};
/// Numbers of the players' controls in a saved state ([`LabWorld::words`]).
const WORDS: usize = 15;
/// Ticks between the digests a recording keeps.
const RECORD_EVERY: u64 = 60;
/// `--net`: ticks between the server's snapshots, the share of packets lost, ticks between the
/// bot's throws.
const SNAPSHOT_EVERY: u64 = 6;
const NET_LOSS: f64 = 0.02;
const BOT_EVERY: u64 = 150;

/// The props the lab draws, in this order: the floor, the block, the barrel, the rocks, the
/// ball, then the sea's: the crate, the log, the pillar, the deck, the boat; then the
/// playground's: the stairs' slab, the ramp, the platform, the player and its visor; then the
/// car's body and wheel; then the aeroplane, its propeller, the runway and the field.
pub(crate) fn props() -> Vec<PropSpec> {
    let mut props = drop_props();
    props.extend(sea::props());
    props.extend(walk::props());
    props.extend(drive::props());
    props.extend(fly::props());
    props.extend(wall::props());
    props.extend(creatures::props());
    props.extend(flood::props());
    props.extend(dominoes::props());
    props
}

/// The pyramid's props.
fn drop_props() -> Vec<PropSpec> {
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

/// What a player asks of the lab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LabCommand {
    /// A ball from `from` along `forward` (unit), at the throwing speed.
    Throw {
        /// Where it leaves, metres.
        from: [f32; 3],
        /// Its way.
        forward: [f32; 3],
    },
    /// Everything back where the scene started (the clock runs on).
    Reset,
    /// The boat's motor: its throttle (−1 astern to 1 ahead) and rudder (−1 to 1), held until
    /// the next.
    Steer {
        /// −1 to 1.
        throttle: f32,
        /// −1 to 1.
        rudder: f32,
    },
    /// The player's walk: m/s along the ground (world x and z), held until the next.
    Walk {
        /// x and z, m/s.
        velocity: [f32; 2],
    },
    /// The player jumps, if on firm ground.
    Jump,
    /// The car's handbrake on or off, held until the next.
    Handbrake {
        /// Pulled.
        on: bool,
    },
    /// The aeroplane's controls, held until the next: throttle (0 to 1), elevator (−1 pull
    /// to 1 push), ailerons (−1 left to 1 right), rudder (−1 left to 1 right).
    Fly {
        /// The four, in that order.
        controls: [f32; 4],
    },
    /// The wrecking ball let go (#142).
    Release,
    /// The creatures' motors let go, or powered again (#143), held until the next.
    Limp {
        /// Let go.
        on: bool,
    },
}

impl Codec for LabCommand {
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Throw { from, forward } => {
                out.push(0);
                for v in from.iter().chain(forward) {
                    out.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            Self::Reset => out.push(1),
            Self::Steer { throttle, rudder } => {
                out.push(2);
                out.extend_from_slice(&throttle.to_bits().to_le_bytes());
                out.extend_from_slice(&rudder.to_bits().to_le_bytes());
            }
            Self::Walk { velocity } => {
                out.push(3);
                for v in velocity {
                    out.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            Self::Jump => out.push(4),
            Self::Handbrake { on } => out.extend_from_slice(&[5, u8::from(*on)]),
            Self::Fly { controls } => {
                out.push(6);
                for v in controls {
                    out.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            Self::Release => out.push(7),
            Self::Limp { on } => out.extend_from_slice(&[8, u8::from(*on)]),
        }
    }

    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        let (&tag, rest) = bytes.split_first()?;
        *bytes = rest;
        match tag {
            0 => {
                let mut v = [0.0_f32; 6];
                for x in &mut v {
                    let (word, rest) = bytes.split_first_chunk::<4>()?;
                    *x = f32::from_bits(u32::from_le_bytes(*word));
                    *bytes = rest;
                }
                Some(Self::Throw {
                    from: [v[0], v[1], v[2]],
                    forward: [v[3], v[4], v[5]],
                })
            }
            1 => Some(Self::Reset),
            2 => {
                let mut v = [0.0_f32; 2];
                for x in &mut v {
                    let (word, rest) = bytes.split_first_chunk::<4>()?;
                    *x = f32::from_bits(u32::from_le_bytes(*word));
                    *bytes = rest;
                }
                Some(Self::Steer {
                    throttle: v[0],
                    rudder: v[1],
                })
            }
            3 => {
                let mut v = [0.0_f32; 2];
                for x in &mut v {
                    let (word, rest) = bytes.split_first_chunk::<4>()?;
                    *x = f32::from_bits(u32::from_le_bytes(*word));
                    *bytes = rest;
                }
                Some(Self::Walk { velocity: v })
            }
            4 => Some(Self::Jump),
            5 => {
                let (&on, rest) = bytes.split_first()?;
                *bytes = rest;
                Some(Self::Handbrake { on: on != 0 })
            }
            6 => {
                let mut v = [0.0_f32; 4];
                for x in &mut v {
                    let (word, rest) = bytes.split_first_chunk::<4>()?;
                    *x = f32::from_bits(u32::from_le_bytes(*word));
                    *bytes = rest;
                }
                Some(Self::Fly { controls: v })
            }
            7 => Some(Self::Release),
            8 => {
                let (&on, rest) = bytes.split_first()?;
                *bytes = rest;
                Some(Self::Limp { on: on != 0 })
            }
            _ => None,
        }
    }
}

/// The flood's water for the water pass (`forge_render::WaterPool`): its first sample (world x,
/// z), metres between samples, samples along x and z, and per sample the surface, the depth and
/// the velocity.
pub(crate) struct PoolView {
    pub origin: [f32; 2],
    pub spacing: f32,
    pub size: [u32; 2],
    pub samples: Vec<[f32; 4]>,
}

/// The bodies of one mesh, in the movers' order: the prop drawing them and how many.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Group {
    /// Its index in [`props`].
    pub prop: usize,
    /// Bodies.
    pub count: u32,
}

/// The lab's world as the clock drives it: Jolt's world, its bodies in the movers' order, the
/// balls the players throw, what the water pushes and the boat, and the state the scene
/// started from.
pub(crate) struct LabWorld {
    world: World,
    bodies: Vec<BodyId>,
    thrown: Vec<BodyId>,
    next_throw: u32,
    tick: u64,
    start: Vec<u8>,
    /// The sea scene's: the waves, the bodies they push and their hulls, the boat.
    sea: Option<sea::Sea>,
    floaters: Vec<sea::Floater>,
    hulls: Vec<Hull>,
    boat: Option<sea::Boat>,
    /// The playground's: the player's input and character, and the shuttling platform.
    player: walk::Player,
    /// The track's: the car and its driver's controls.
    driver: drive::Driver,
    /// The field's: the aeroplane and its pilot's controls.
    pilot: fly::Pilot,
    /// The wall's mortar and the wrecking ball.
    wall: Option<wall::Wall>,
    /// The creatures, and whether their motors are let go.
    herd: Option<creatures::Herd>,
    /// The flood's water (the authoritative column model) and its dam.
    water: Option<forge_physics::shallow::Pool>,
    dam: Option<flood::Dam>,
    water_start: Option<forge_physics::shallow::Pool>,
    /// The domino run.
    run: Option<dominoes::Run>,
    limp: bool,
    platform: Option<BodyId>,
    /// The workers the waves and the pushes are worked out on.
    pool: Arc<TaskPool>,
}

/// What a scene draws: its bodies' groups (the movers) and its still props with their
/// transforms.
pub(crate) struct Layout {
    pub groups: Vec<Group>,
    pub statics: Vec<(usize, Mat4)>,
}

impl LabWorld {
    /// The scene `kind` from scratch: the same calls in the same order every time, so two built
    /// alike are the same world to the bit. Also what the scene draws.
    pub(crate) fn new(kind: LabScene, pool: Arc<TaskPool>) -> Result<(Self, Layout)> {
        let mut world = World::new(&WorldDesc {
            // The client's rule (D-005): the cores less two, the caller one of them.
            threads: (std::thread::available_parallelism().map_or(4, |n| n.get()) / 2)
                .saturating_sub(3)
                .max(1) as u32,
            ..WorldDesc::default()
        });
        // The floor: under the pyramid at 0, under the sea 12 m down.
        let floor_y = match kind {
            LabScene::Drop => 0.0,
            LabScene::Sea => sea::FLOOR_Y,
            LabScene::Walk
            | LabScene::Drive
            | LabScene::Fly
            | LabScene::Break
            | LabScene::Creatures
            | LabScene::Flood
            | LabScene::Dominoes => 0.0,
        };
        // The flight's is a field of grass, wide enough to fly over for a while.
        let (floor, floor_half) = match kind {
            LabScene::Fly => (FIELD, fly::FIELD_HALF),
            _ => (FLOOR, FLOOR_HALF),
        };
        let floor_at = Vec3::new(0.0, floor_y - 0.5, 0.0);
        let floor_shape = Shape::cuboid(Vec3::new(floor_half, 0.5, floor_half), 0.05, 0.0)?;
        world.add_body(&BodyDesc::fixed(&floor_shape, floor_at.as_dvec3()))?;
        let mut statics = vec![(floor, Mat4::from_translation(floor_at))];
        // The shapes, each with its origin where its mesh has its own: the barrel's and the
        // ball's at their bottom, the rocks' hulls from their meshes' vertices.
        let block_shape = Shape::cuboid(Vec3::splat(BLOCK_HALF), 0.03, 2300.0)?;
        let half_length = super::BARREL_LENGTH * 0.5;
        let up = |h: f32| Vec3::new(0.0, h, 0.0);
        let barrel_shape = Shape::cylinder(half_length, super::BARREL_RADIUS, 0.03, 300.0)?
            .offset(up(half_length), Quat::IDENTITY)?;
        let ball_shape =
            Shape::sphere(BALL_RADIUS, 500.0)?.offset(up(BALL_RADIUS), Quat::IDENTITY)?;
        let props = props();
        let rock_shapes = ROCKS
            .iter()
            .map(|&k| {
                let mesh = props[k].generate();
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
        // Turned and spinning at random, without trigonometry.
        let spin = |k: u64| {
            Vec3::new(
                (2.0 * unit(k * 7 + 1) - 1.0) as f32,
                (2.0 * unit(k * 7 + 2) - 1.0) as f32,
                (2.0 * unit(k * 7 + 3) - 1.0) as f32,
            ) * 3.0
        };
        // A point over a disc of `radius` round (x, z), `low` to `high` up: a point of the
        // square folded onto the circle by its length (a square root only).
        let scatter = |k: u64, centre: [f64; 2], radius: f64, low: f64, high: f64| {
            let (r, h) = (unit(3 * k), unit(3 * k + 2));
            let (x, z) = (
                2.0 * unit(3 * k + 1) - 1.0,
                2.0 * unit(3 * k + 1_000_003) - 1.0,
            );
            let len = (x * x + z * z).sqrt().max(1e-6);
            let reach = radius * r.sqrt();
            DVec3::new(
                centre[0] + x / len * reach,
                low + (high - low) * h,
                centre[1] + z / len * reach,
            )
        };

        let mut bodies = Vec::new();
        let mut groups = Vec::new();
        let mut group = |prop: usize, ids: Vec<BodyId>, bodies: &mut Vec<BodyId>| {
            groups.push(Group {
                prop,
                count: ids.len() as u32,
            });
            bodies.extend(ids);
        };
        let mut floaters = Vec::new();
        let mut hulls = Vec::new();
        let mut boat = None;
        let mut player = walk::Player::default();
        let mut platform = None;
        let mut driver = drive::Driver::default();
        let mut pilot = fly::Pilot::default();
        let mut wall = None;
        let mut herd = None;
        let mut water = None;
        let mut dam = None;
        let mut run = None;
        let mut k = 1_000u64;
        let mut balls = Vec::new();
        match kind {
            LabScene::Drop => {
                // The pyramid: layer L has (8 − L)² blocks, 2 cm apart, each a hair over the
                // one under it.
                let mut blocks = Vec::new();
                let pitch = f64::from(2.0 * BLOCK_HALF + 0.02);
                for layer in 0..PYRAMID_LAYERS {
                    let side = PYRAMID_LAYERS - layer;
                    let offset = |k: u32| (f64::from(k) - f64::from(side - 1) * 0.5) * pitch;
                    let y = f64::from(BLOCK_HALF)
                        + f64::from(layer) * f64::from(2.0 * BLOCK_HALF + 0.001);
                    for j in 0..side {
                        for i in 0..side {
                            blocks.push(world.add_body(&BodyDesc {
                                friction: 0.7,
                                ..BodyDesc::dynamic(
                                    &block_shape,
                                    DVec3::new(offset(i), y, offset(j)),
                                )
                            })?);
                        }
                    }
                }
                group(BLOCK, blocks, &mut bodies);
                // The rain: barrels, rocks and balls over a disc of 10 m around the pyramid,
                // from 8 to 45 m up.
                let rain = |k: u64| scatter(k, [0.0, 0.0], 10.0, 8.0, 45.0);
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
                group(BARREL, barrels, &mut bodies);
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
                for (size, rocks) in by_size.into_iter().enumerate() {
                    group(ROCKS[size], rocks, &mut bodies);
                }
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
            }
            LabScene::Sea => {
                // The jetty: a deck on two rows of pillars, running out along −z from the
                // shore's side.
                let deck_y = sea::DECK_TOP - sea::DECK_HALF[1];
                let deck_at = Vec3::new(0.0, deck_y, 0.0);
                let deck_shape = Shape::cuboid(Vec3::from(sea::DECK_HALF), 0.04, 0.0)?;
                world.add_body(&BodyDesc::fixed(&deck_shape, deck_at.as_dvec3()))?;
                statics.push((DECK, Mat4::from_translation(deck_at)));
                let pillar_half = 0.5 * (sea::DECK_TOP - sea::FLOOR_Y);
                let pillar_shape = Shape::cuboid(
                    Vec3::new(sea::PILLAR_HALF, pillar_half, sea::PILLAR_HALF),
                    0.03,
                    0.0,
                )?;
                for side in [-1.0_f32, 1.0] {
                    for n in 0..sea::PILLARS {
                        let z = -sea::DECK_HALF[2]
                            + 1.0
                            + (2.0 * sea::DECK_HALF[2] - 2.0) * n as f32
                                / (sea::PILLARS - 1) as f32;
                        let at = Vec3::new(
                            side * (sea::DECK_HALF[0] - 0.4),
                            sea::FLOOR_Y + pillar_half,
                            z,
                        );
                        world.add_body(&BodyDesc::fixed(&pillar_shape, at.as_dvec3()))?;
                        statics.push((PILLAR, Mat4::from_translation(at)));
                    }
                }
                // What floats, dropped from 1 to 6 m over 11 m of water beyond the jetty's end,
                // and the rocks that sink; each with its hull.
                let around = [9.0, -25.0];
                let fall = |k: u64| scatter(k, around, 11.0, 1.0, 6.0);
                let mut afloat = |hull: Hull, ids: &[BodyId], hulls: &mut Vec<Hull>| {
                    hulls.push(hull);
                    floaters.extend(ids.iter().map(|&body| sea::Floater {
                        body,
                        hull: hulls.len() - 1,
                    }));
                };
                let crate_shape = Shape::cuboid(Vec3::splat(sea::CRATE_HALF), 0.025, 600.0)?;
                let mut crates = Vec::new();
                for _ in 0..SEA_CRATES {
                    k += 1;
                    crates.push(world.add_body(&BodyDesc {
                        rotation: rotation(k),
                        angular_velocity: spin(k),
                        friction: 0.6,
                        ..BodyDesc::dynamic(&crate_shape, fall(k))
                    })?);
                }
                afloat(
                    Hull::cuboid(Vec3::splat(sea::CRATE_HALF), 3),
                    &crates,
                    &mut hulls,
                );
                group(CRATE, crates, &mut bodies);
                let mut barrels = Vec::new();
                for _ in 0..SEA_BARRELS {
                    k += 1;
                    barrels.push(world.add_body(&BodyDesc {
                        rotation: rotation(k),
                        angular_velocity: spin(k),
                        friction: 0.5,
                        restitution: 0.2,
                        mass: Some(60.0),
                        ..BodyDesc::dynamic(&barrel_shape, fall(k))
                    })?);
                }
                afloat(
                    Hull::cylinder(super::BARREL_RADIUS, super::BARREL_LENGTH, 0.0, 16, 3),
                    &barrels,
                    &mut hulls,
                );
                group(BARREL, barrels, &mut bodies);
                let log_half = sea::LOG_LENGTH * 0.5;
                let log_shape = Shape::cylinder(log_half, sea::LOG_RADIUS, 0.03, 700.0)?
                    .offset(up(log_half), Quat::IDENTITY)?;
                let mut logs = Vec::new();
                for _ in 0..SEA_LOGS {
                    k += 1;
                    logs.push(world.add_body(&BodyDesc {
                        rotation: rotation(k),
                        angular_velocity: spin(k),
                        friction: 0.7,
                        ..BodyDesc::dynamic(&log_shape, fall(k))
                    })?);
                }
                afloat(
                    Hull::cylinder(sea::LOG_RADIUS, sea::LOG_LENGTH, 0.0, 12, 6),
                    &logs,
                    &mut hulls,
                );
                group(LOG, logs, &mut bodies);
                let mut by_size: [Vec<BodyId>; 3] = Default::default();
                for n in 0..SEA_ROCKS {
                    k += 1;
                    let size = n as usize % 3;
                    by_size[size].push(world.add_body(&BodyDesc {
                        rotation: rotation(k),
                        friction: 0.8,
                        ..BodyDesc::dynamic(&rock_shapes[size], fall(k))
                    })?);
                }
                for (size, rocks) in by_size.into_iter().enumerate() {
                    // A rock's hull: a ball of most of its radius about its middle.
                    let r = ROCK_RADII[size];
                    afloat(
                        Hull::sphere(0.8 * r, Vec3::new(0.0, 0.6 * r, 0.0), 3),
                        &rocks,
                        &mut hulls,
                    );
                    group(ROCKS[size], rocks, &mut bodies);
                }
                for _ in 0..SEA_BALLS {
                    k += 1;
                    balls.push(world.add_body(&BodyDesc {
                        angular_velocity: spin(k),
                        friction: 0.6,
                        restitution: 0.6,
                        ..BodyDesc::dynamic(&ball_shape, fall(k))
                    })?);
                }
            }
            LabScene::Walk => {
                let ground = walk::build(&mut world, SLAB, RAMP, &block_shape, BLOCK_HALF)?;
                statics.extend(ground.statics);
                group(BLOCK, ground.blocks, &mut bodies);
                let mut crates = ground.light;
                crates.extend(ground.heavy);
                group(CRATE, crates, &mut bodies);
                group(PLATFORM, vec![ground.platform], &mut bodies);
                platform = Some(ground.platform);
                player.character = Some(ground.player);
            }
            LabScene::Drive => {
                let crate_shape = Shape::cuboid(Vec3::splat(sea::CRATE_HALF), 0.025, 150.0)?;
                let track = drive::build(
                    &mut world,
                    RAMP,
                    walk::RAMP_HALF,
                    &barrel_shape,
                    &crate_shape,
                    sea::CRATE_HALF,
                )?;
                statics.extend(track.statics);
                group(BARREL, track.barrels, &mut bodies);
                group(CRATE, track.crates, &mut bodies);
                group(CAR, vec![track.chassis], &mut bodies);
                driver.car = Some((track.chassis, track.vehicle));
            }
            LabScene::Fly => {
                let field = fly::build(&mut world, RUNWAY)?;
                statics.extend(field.statics);
                group(PLANE, vec![field.plane], &mut bodies);
                pilot.plane = Some(field.plane);
            }
            LabScene::Break => {
                let site = wall::build(&mut world, POST, BEAM)?;
                statics.extend(site.statics);
                group(BRICK, site.bricks, &mut bodies);
                group(WRECKING_BALL, vec![site.ball], &mut bodies);
                group(COLUMN, vec![site.wall.column], &mut bodies);
                for (k, &(piece, _)) in site.wall.pieces.iter().enumerate() {
                    group(PIECE + k, vec![piece], &mut bodies);
                }
                wall = Some(site.wall);
            }
            LabScene::Creatures => {
                let field = creatures::build(&mut world, POLE)?;
                statics.extend(field.statics);
                for (k, part) in field.parts.into_iter().enumerate() {
                    group(CREATURE + k, part, &mut bodies);
                }
                herd = Some(field.herd);
            }
            LabScene::Flood => {
                let basin = flood::build(
                    &mut world,
                    &flood::Props {
                        wall_x: FLOOD,
                        wall_z: FLOOD + 1,
                        block: FLOOD + 3,
                        hut: FLOOD + 4,
                    },
                )?;
                statics.extend(basin.statics);
                group(FLOOD + 2, vec![basin.gate], &mut bodies);
                // What floats: crates, barrels and logs, two thirds afloat behind the gate and
                // a third lying on the floor beyond it.
                let crate_shape = Shape::cuboid(Vec3::splat(sea::CRATE_HALF), 0.025, 600.0)?;
                let log_half = sea::LOG_LENGTH * 0.5;
                let log_shape = Shape::cylinder(log_half, sea::LOG_RADIUS, 0.03, 700.0)?
                    .offset(up(log_half), Quat::IDENTITY)?;
                let kinds = [
                    (
                        CRATE,
                        &crate_shape,
                        FLOOD_CRATES,
                        Hull::cuboid(Vec3::splat(sea::CRATE_HALF), 3),
                        None,
                    ),
                    (
                        BARREL,
                        &barrel_shape,
                        FLOOD_BARRELS,
                        Hull::cylinder(super::BARREL_RADIUS, super::BARREL_LENGTH, 0.0, 16, 3),
                        Some(60.0),
                    ),
                    (
                        LOG,
                        &log_shape,
                        FLOOD_LOGS,
                        Hull::cylinder(sea::LOG_RADIUS, sea::LOG_LENGTH, 0.0, 12, 6),
                        None,
                    ),
                ];
                for (prop, shape, count, hull, mass) in kinds {
                    hulls.push(hull);
                    let mut ids = Vec::new();
                    for _ in 0..count {
                        k += 1;
                        let body = world.add_body(&BodyDesc {
                            rotation: rotation(k),
                            friction: 0.6,
                            mass,
                            ..BodyDesc::dynamic(shape, flood::afloat_at(k, unit))
                        })?;
                        floaters.push(sea::Floater {
                            body,
                            hull: hulls.len() - 1,
                        });
                        ids.push(body);
                    }
                    group(prop, ids, &mut bodies);
                }
                water = Some(basin.pool);
                dam = Some(basin.dam);
            }
            LabScene::Dominoes => {
                let spiral = dominoes::build(&mut world)?;
                group(DOMINO, spiral.dominoes.clone(), &mut bodies);
                run = Some(spiral);
            }
        }
        // The balls to throw, asleep out of sight until thrown, after the scene's.
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
        balls.extend(&thrown);
        if kind == LabScene::Sea {
            hulls.push(Hull::sphere(BALL_RADIUS, up(BALL_RADIUS), 4));
            let hull = hulls.len() - 1;
            floaters.extend(balls.iter().map(|&body| sea::Floater { body, hull }));
        }
        group(BALL, balls, &mut bodies);
        if kind == LabScene::Sea {
            let (shape, hull) = sea::boat_shapes()?;
            let at = sea::boat_start();
            let body = world.add_body(&BodyDesc {
                rotation: at.rotation,
                friction: 0.5,
                mass: Some(sea::boat_mass()),
                ..BodyDesc::dynamic(&shape, at.position)
            })?;
            hulls.push(hull);
            floaters.push(sea::Floater {
                body,
                hull: hulls.len() - 1,
            });
            group(BOAT, vec![body], &mut bodies);
            boat = Some(sea::boat_still(body));
        }
        // The aeroplane's propeller after them.
        if pilot.plane.is_some() {
            groups.push(Group {
                prop: PROPELLER,
                count: 1,
            });
        }
        // The car's four wheels after the bodies' movers.
        if driver.car.is_some() {
            groups.push(Group {
                prop: WHEEL,
                count: 4,
            });
        }
        // The wrecking ball's chain after the bodies.
        if wall.is_some() {
            groups.push(Group {
                prop: CHAIN,
                count: 1,
            });
        }
        // The player, drawn by two movers after the bodies': its capsule and its visor.
        if player.character.is_some() {
            groups.push(Group {
                prop: PLAYER,
                count: 1,
            });
            groups.push(Group {
                prop: VISOR,
                count: 1,
            });
        }
        world.optimize_broad_phase();
        let start = world.save_state();
        Ok((
            Self {
                world,
                bodies,
                thrown,
                next_throw: 0,
                tick: 0,
                start,
                sea: (kind == LabScene::Sea).then(sea::Sea::new),
                floaters,
                hulls,
                boat,
                player,
                platform,
                driver,
                pilot,
                wall,
                herd,
                water_start: water.clone(),
                water,
                dam,
                run,
                limp: false,
                pool,
            },
            Layout { groups, statics },
        ))
    }

    /// Its bodies' transforms, in the movers' order, into `out`.
    pub(crate) fn transforms(&self, out: &mut Vec<Transform>) {
        self.world.transforms(&self.bodies, out);
        // The car's wheels after the bodies.
        self.driver.wheels(&self.world, out);
        self.pilot.propeller(&self.world, self.tick, out);
        if let Some(wall) = &self.wall {
            wall.chain(&self.world, out);
        }
        // The player's capsule and visor after the bodies, turned where it last walked.
        if let Some(c) = self.player.character {
            let at = Transform {
                position: self.world.character(c).position,
                rotation: self.player.rotation(),
            };
            out.extend([at, at]);
        }
    }

    /// The player as drawn, when the scene has one: its feet and its facing.
    pub(crate) fn player(&self) -> Option<Transform> {
        let c = self.player.character?;
        Some(Transform {
            position: self.world.character(c).position,
            rotation: self.player.rotation(),
        })
    }

    /// Bodies, and those awake.
    pub(crate) fn bodies(&self) -> usize {
        self.bodies.len()
    }

    pub(crate) fn awake(&self) -> u32 {
        self.world.active_bodies()
    }

    /// The players' controls as the state carries them, [`WORDS`] numbers: the boat's
    /// throttle and rudder, the walk, a jump waiting, the player's facing, the car's throttle,
    /// steering and handbrake, the aeroplane's throttle, elevator, ailerons and rudder, the
    /// creatures let go (0 where there is none).
    fn words(&self) -> [f32; WORDS] {
        let (throttle, rudder) = self.boat.map_or((0.0, 0.0), |b| (b.throttle, b.rudder));
        let p = &self.player;
        [
            throttle,
            rudder,
            p.walk[0],
            p.walk[1],
            f32::from(u8::from(p.jump)),
            p.facing[0],
            p.facing[1],
            self.driver.throttle,
            self.driver.steer,
            self.driver.handbrake,
            self.pilot.throttle,
            self.pilot.elevator,
            self.pilot.aileron,
            self.pilot.rudder,
            f32::from(u8::from(self.limp)),
        ]
    }

    /// The boat's transform, when the scene has one.
    #[cfg(test)]
    fn boat(&self) -> Option<Transform> {
        let boat = self.boat?;
        let mut t = Vec::new();
        self.world.transforms(&[boat.body], &mut t);
        t.first().copied()
    }

    /// The mover of what a player rides, the boat, the car or the aeroplane: its body's place among the
    /// bodies, which the movers draw first.
    pub(crate) fn ride(&self) -> Option<usize> {
        let body = self
            .boat
            .map(|b| b.body)
            .or(self.driver.car.map(|c| c.0))
            .or(self.pilot.plane)?;
        self.bodies.iter().position(|&b| b == body)
    }

    /// Whether the scene has the sea.
    pub(crate) fn has_sea(&self) -> bool {
        self.sea.is_some()
    }

    fn throw(&mut self, from: Vec3, forward: Vec3) {
        let ball = self.thrown[self.next_throw as usize];
        self.next_throw = (self.next_throw + 1) % self.thrown.len() as u32;
        let forward = forward.normalize_or(Vec3::NEG_Z);
        let at = (from + forward - Vec3::new(0.0, BALL_RADIUS, 0.0)).as_dvec3();
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
    }
}

impl Simulation for LabWorld {
    type Command = LabCommand;

    fn tick(&mut self, commands: &[Stamped<LabCommand>]) {
        for c in commands {
            match c.command {
                LabCommand::Throw { from, forward } => {
                    self.throw(Vec3::from_array(from), Vec3::from_array(forward));
                }
                LabCommand::Reset => {
                    let start = std::mem::take(&mut self.start);
                    if let Err(e) = self.world.restore_state(&start) {
                        tracing::warn!("the lab's start: {e}");
                    }
                    self.start = start;
                    self.next_throw = 0;
                    if let Some(boat) = &mut self.boat {
                        (boat.throttle, boat.rudder) = (0.0, 0.0);
                    }
                    (
                        self.driver.throttle,
                        self.driver.steer,
                        self.driver.handbrake,
                    ) = (0.0, 0.0, 0.0);
                    let plane = self.pilot.plane;
                    self.pilot = fly::Pilot {
                        plane,
                        ..fly::Pilot::default()
                    };
                    self.limp = false;
                    if let (Some(water), Some(start)) = (&mut self.water, &self.water_start) {
                        water.clone_from(start);
                    }
                }
                LabCommand::Steer { throttle, rudder } => {
                    let (throttle, rudder) = (throttle.clamp(-1.0, 1.0), rudder.clamp(-1.0, 1.0));
                    if let Some(boat) = &mut self.boat {
                        (boat.throttle, boat.rudder) = (throttle, rudder);
                    }
                    if self.driver.car.is_some() {
                        (self.driver.throttle, self.driver.steer) = (throttle, rudder);
                    }
                }
                LabCommand::Handbrake { on } => {
                    self.driver.handbrake = f32::from(u8::from(on));
                }
                LabCommand::Fly { controls } => {
                    let p = &mut self.pilot;
                    p.throttle = controls[0].clamp(0.0, 1.0);
                    p.elevator = controls[1].clamp(-1.0, 1.0);
                    p.aileron = controls[2].clamp(-1.0, 1.0);
                    p.rudder = controls[3].clamp(-1.0, 1.0);
                }
                LabCommand::Walk { velocity } => {
                    let v = Vec2::from(velocity);
                    // No faster than a run, whatever a command says.
                    self.player.walk = v.clamp_length_max(walk::RUN_SPEED).to_array();
                }
                LabCommand::Jump => self.player.jump = true,
                LabCommand::Release => {
                    if let Some(wall) = &self.wall {
                        wall.release(&mut self.world);
                    }
                    if let (Some(dam), Some(water)) = (&self.dam, &mut self.water) {
                        dam.open(&mut self.world, water);
                    }
                    if let Some(run) = &self.run {
                        run.push(&mut self.world);
                    }
                }
                LabCommand::Limp { on } => self.limp = on,
            }
        }
        // The sea at this tick's start: what floats pushed by it, the boat's motor too.
        if let Some(sea) = &self.sea {
            let heights = sea.at(self.tick as f64 * f64::from(TICK), &self.pool);
            sea::float(
                &mut self.world,
                &self.floaters,
                &self.hulls,
                &sea::Surface(&heights),
                &Fluid::SEA,
                &self.pool,
            );
            if let Some(boat) = &self.boat {
                boat.drive(&mut self.world, &heights);
            }
        }
        // The flood's water a tick on, then what floats pushed by it.
        if let Some(water) = &mut self.water {
            water.step(TICK);
            sea::float(
                &mut self.world,
                &self.floaters,
                &self.hulls,
                &*water,
                &FRESH,
                &self.pool,
            );
        }
        // The playground's platform and player, before the bodies move.
        self.player
            .tick(&mut self.world, self.platform, self.tick, TICK);
        self.driver.tick(&mut self.world);
        self.pilot.tick(&mut self.world);
        let column = self.wall.as_ref().map(|w| w.column_velocity(&self.world));
        // The creatures' motors driven to their poses at this tick (or let go).
        if let Some(herd) = &self.herd {
            herd.drive(
                &mut self.world,
                self.tick as f64 * f64::from(TICK),
                self.limp,
            );
        }
        if let Err(e) = self.world.step(TICK, 1) {
            tracing::warn!("physics tick {}: {e}", self.tick);
        }
        // A mannequin knocked hard enough comes off its stand.
        if let Some(herd) = &self.herd {
            herd.knock(&mut self.world, TICK);
        }
        // The lifted gate stops at its top.
        if let Some(dam) = &self.dam {
            dam.tick(&mut self.world);
        }
        // The mortar that carried more than it holds in that step breaks, and the column
        // shatters under a blow.
        if let (Some(wall), Some(before)) = (&self.wall, column) {
            wall.crack(&mut self.world, TICK);
            wall.shatter(&mut self.world, before, TICK);
        }
        self.tick += 1;
    }

    fn now(&self) -> u64 {
        self.tick
    }

    fn save(&mut self) -> Vec<u8> {
        let mut out = self.tick.to_le_bytes().to_vec();
        out.extend_from_slice(&self.next_throw.to_le_bytes());
        for word in self.words() {
            out.extend_from_slice(&word.to_bits().to_le_bytes());
        }
        // The flood's water, before the world's state (whose length Jolt knows).
        if let Some(water) = &self.water {
            water.save(&mut out);
        }
        out.extend(self.world.save_state());
        out
    }

    fn restore(&mut self, state: &[u8]) {
        let (tick, rest) = state.split_at(8);
        let (next, rest) = rest.split_at(4);
        let (words, world) = rest.split_at(4 * WORDS);
        self.tick = u64::from_le_bytes(tick.try_into().expect("8 bytes"));
        self.next_throw = u32::from_le_bytes(next.try_into().expect("4 bytes"));
        let w: Vec<f32> = words
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f32::from_bits(u32::from_le_bytes(b)))
            .collect();
        if let Some(boat) = &mut self.boat {
            (boat.throttle, boat.rudder) = (w[0], w[1]);
        }
        self.player.walk = [w[2], w[3]];
        self.player.jump = w[4] != 0.0;
        self.player.facing = [w[5], w[6]];
        (
            self.driver.throttle,
            self.driver.steer,
            self.driver.handbrake,
        ) = (w[7], w[8], w[9]);
        self.pilot.throttle = w[10];
        self.pilot.elevator = w[11];
        self.pilot.aileron = w[12];
        self.pilot.rudder = w[13];
        self.limp = w[14] != 0.0;
        let world = match &mut self.water {
            Some(water) => &world[water.restore(world)..],
            None => world,
        };
        if let Err(e) = self.world.restore_state(world) {
            tracing::warn!("a lab state: {e}");
        }
    }

    fn digest(&mut self) -> u64 {
        let mut d = self.world.digest(&self.bodies)
            ^ self.tick.rotate_left(17)
            ^ u64::from(self.next_throw);
        for (k, word) in self.words().into_iter().enumerate() {
            d ^= u64::from(word.to_bits()).rotate_left(7 * k as u32 + 3);
        }
        // The player's character, where it stands and how it moves, to the bit.
        if let Some(c) = self.player.character {
            let s = self.world.character(c);
            for (k, p) in s.position.to_array().into_iter().enumerate() {
                d ^= p.to_bits().rotate_left(11 * k as u32 + 5);
            }
            for (k, v) in s.velocity.to_array().into_iter().enumerate() {
                d ^= u64::from(v.to_bits()).rotate_left(13 * k as u32 + 1);
            }
        }
        // The flood's water, every column and pipe to the bit.
        if let Some(water) = &self.water {
            d ^= water.digest().rotate_left(29);
        }
        d
    }
}

/// `--net`: the server, this player's client and the bot's, and their links.
struct Net {
    server: Server<LabWorld>,
    client: Client<LabWorld>,
    bot: Client<LabWorld>,
    up: [Link<InputPacket<LabCommand>>; 2],
    down: [Link<Snapshot>; 2],
    /// One way, milliseconds.
    delay_ms: f64,
    /// Milliseconds the client's corrections took since the last title, and the most.
    correction_ms: Vec<f64>,
    worst_correction_ms: f64,
    /// Whether the bot plays.
    bot_on: bool,
}

impl Net {
    fn new(kind: LabScene, delay_ms: f64, pool: &Arc<TaskPool>) -> Result<Self> {
        let lead = (delay_ms / 1e3 / f64::from(TICK)).ceil() as u64 + 2;
        // A client runs ahead through its own steps, so it knows the digests of those ticks.
        let ahead = |player: PlayerId| -> Result<Client<LabWorld>> {
            let mut client = Client::new(LabWorld::new(kind, Arc::clone(pool))?.0, player);
            for _ in 0..lead {
                client.step();
            }
            Ok(client)
        };
        let link = |seed| LinkParams::new(delay_ms, NET_LOSS, seed);
        Ok(Self {
            server: Server::new(LabWorld::new(kind, Arc::clone(pool))?.0),
            client: ahead(0)?,
            bot: ahead(1)?,
            up: [Link::new(link(10)), Link::new(link(11))],
            down: [Link::new(link(20)), Link::new(link(21))],
            delay_ms,
            correction_ms: Vec::new(),
            worst_correction_ms: 0.0,
            bot_on: true,
        })
    }

    /// One tick of the whole session: the server takes what arrived, steps and sometimes sends
    /// its snapshot; each client takes what arrived, steps with its own commands and sends them.
    fn tick(&mut self, mine: &mut Vec<LabCommand>) {
        let now = self.server.sim.now() as f64 / f64::from(TICK_RATE);
        for link in &mut self.up {
            for packet in link.receive(now) {
                self.server.receive(&packet);
            }
        }
        self.server.step();
        if self.server.sim.now().is_multiple_of(SNAPSHOT_EVERY) {
            let snapshot = self.server.snapshot();
            let bytes = snapshot.state.len();
            for link in &mut self.down {
                link.send(now, snapshot.clone(), bytes);
            }
        }
        // The bot throws at the pyramid from one of eight places round it, every 2.5 s.
        let bot_tick = self.bot.sim.now();
        if self.bot_on && bot_tick > 0 && bot_tick.is_multiple_of(BOT_EVERY) {
            const ROUND: [[f32; 2]; 8] = [
                [14.0, 0.0],
                [10.0, 10.0],
                [0.0, 14.0],
                [-10.0, 10.0],
                [-14.0, 0.0],
                [-10.0, -10.0],
                [0.0, -14.0],
                [10.0, -10.0],
            ];
            let [x, z] = ROUND[(bot_tick / BOT_EVERY % 8) as usize];
            let from = Vec3::new(x, 3.0, z);
            let forward = (Vec3::new(0.0, 4.0, 0.0) - from).normalize();
            self.bot.command(LabCommand::Throw {
                from: from.to_array(),
                forward: forward.to_array(),
            });
        }
        for c in mine.drain(..) {
            self.client.command(c);
        }
        for (k, client) in [&mut self.client, &mut self.bot].into_iter().enumerate() {
            for snapshot in self.down[k].receive(now) {
                let start = Instant::now();
                let corrected = client.stats.corrected;
                client.apply(&snapshot);
                if k == 0 && client.stats.corrected > corrected {
                    let ms = start.elapsed().as_secs_f64() * 1e3;
                    self.correction_ms.push(ms);
                    self.worst_correction_ms = self.worst_correction_ms.max(ms);
                }
            }
            client.step();
            if let Some(packet) = client.outgoing() {
                self.up[k].send(now, packet, 64);
            }
        }
    }

    fn title(&mut self) -> String {
        let s = self.client.stats;
        let n = self.correction_ms.len();
        let mean = self.correction_ms.iter().sum::<f64>() / n.max(1) as f64;
        self.correction_ms.clear();
        format!(
            "net {:.0} ms, {:.0} % lost: {} snapshots, {} predicted to the bit, {} corrected ({} ticks run again; {mean:.1} ms each lately, at most {:.1})",
            self.delay_ms,
            NET_LOSS * 100.0,
            s.snapshots,
            s.matched,
            s.corrected,
            s.replayed_ticks,
            self.worst_correction_ms,
        )
    }
}

/// How the lab runs.
enum Mode {
    /// The world in this process alone, its commands recorded and maybe replayed.
    Local {
        world: Box<LabWorld>,
        recording: Recording<LabCommand>,
        replay: Option<Box<Replay>>,
    },
    /// Through a server and a client (`--net`).
    Net(Box<Net>),
}

/// `--replay`: a recording's commands fed at their ticks, its digests checked.
struct Replay {
    recording: Recording<LabCommand>,
    next: usize,
    checked: usize,
    diverged: Option<u64>,
}

/// The lab's running state: the world, how it runs, the transforms of the last two ticks.
pub(crate) struct Lab {
    mode: Mode,
    /// Commands made since the last tick (Space, Enter).
    queued: Vec<LabCommand>,
    previous: Vec<Transform>,
    current: Vec<Transform>,
    /// Seconds not yet ticked.
    pending: f32,
    /// Milliseconds the ticks took since the last title, and over the run.
    tick_ms: Vec<f64>,
    run_tick_ms: Vec<f64>,
    /// Bodies awake at the last tick.
    awake: u32,
    /// Ticks at which the hash is logged.
    logged: [u64; 5],
    /// `--record`: where the session goes at exit.
    record: Option<PathBuf>,
}

/// The sea scene's water (#138): the island's cascades of FFT waves from the same seed as the
/// physics' heights, and an open surface with no shore.
pub(crate) fn water(
    ctx: &Context,
) -> Result<(
    forge_render::WaterCascades,
    forge_render::WaterSurface,
    Vec<forge_procgen::Ocean>,
)> {
    let oceans = sea::oceans();
    let descs: Vec<forge_render::WaterCascadeDesc> = oceans
        .iter()
        .map(|o| {
            let omega = o.mean_frequency();
            forge_render::WaterCascadeDesc {
                patch: o.params.patch as f32,
                choppiness: o.params.choppiness as f32,
                samples: o.gpu_samples(),
                omega: omega as f32,
                shelf: forge_procgen::tma(omega, o.params.depth) as f32,
            }
        })
        .collect();
    let cascades = forge_render::WaterCascades::new(&ctx.device, &ctx.shaders, &descs)?;
    let surface = forge_render::WaterSurface::new(&ctx.device, &ctx.shaders, None)?;
    Ok((cascades, surface, oceans))
}

/// Builds the lab's scene and its world: the floor, the bodies of `kind`, the movers that draw
/// them.
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
    let mut materials = CityMaterials::new(&ctx.device)?;
    // The boat's rows, one per material of its model in order: its mesh's sections.
    let (model, _) = sea::boat_model();
    let boat = model.mesh("boat").context("the model's boat")?;
    materials.add_rows(
        "lab-boat",
        boat.materials
            .iter()
            .map(|m| (m.name.clone(), super::model_layer(m)))
            .collect(),
    );
    // The car's body and its wheel, and the aeroplane and its propeller, the same way (#140,
    // #141).
    let (car, _) = drive::car_model();
    let (plane, _) = fly::plane_model();
    for (model, prop, mesh) in [
        (car, "lab-car", "car"),
        (car, "lab-wheel", "car-wheel"),
        (plane, "lab-plane", "plane"),
        (plane, "lab-propeller", "plane-prop"),
    ] {
        let mesh = model.mesh(mesh).context("a model's mesh")?;
        materials.add_rows(
            prop,
            mesh.materials
                .iter()
                .map(|m| (m.name.clone(), super::model_layer(m)))
                .collect(),
        );
    }
    // The creatures' parts, a kind's rows shared by its parts (#143).
    for (prop, kind) in creatures::materials() {
        materials.add_rows(
            prop,
            kind.iter()
                .map(|m| (m.name.clone(), super::model_layer(m)))
                .collect(),
        );
    }
    materials.apply(&mut builder, &props, &ids);
    builder.set_ray_traced(!args.no_shadows);
    builder.set_origin(scene_origin(args));
    let pool = Arc::new(TaskPool::client());
    let (world, layout) = LabWorld::new(kind, Arc::clone(&pool))?;
    for &(prop, at) in &layout.statics {
        builder.add_instance(ids[prop], at);
    }
    let per_mesh: Vec<(MeshId, u32)> = layout
        .groups
        .iter()
        .map(|g| (ids[g.prop], g.count))
        .collect();
    builder.reserve_movers(&per_mesh);
    let mut scene = builder.build(&ctx.device)?;
    scene.build_tlas(&ctx.device, &ctx.shaders)?;

    let mut current = Vec::new();
    world.transforms(&mut current);
    tracing::info!(
        scene = ?kind,
        bodies = world.bodies(),
        threads = world.world.threads(),
        state_kib = world.start.len() / 1024,
        ms = start.elapsed().as_millis(),
        "physics lab ready"
    );
    let mode = if let Some(delay_ms) = args.net {
        anyhow::ensure!(
            args.record.is_none() && args.replay.is_none(),
            "--net records and replays nothing"
        );
        tracing::info!(
            delay_ms,
            loss = NET_LOSS,
            snapshot_every = SNAPSHOT_EVERY,
            "the lab through a server and a client, a bot throwing too"
        );
        Mode::Net(Box::new(Net::new(kind, delay_ms, &pool)?))
    } else {
        let replay = args
            .replay
            .as_ref()
            .map(|path| -> Result<Replay> {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("reading the recording {}", path.display()))?;
                let recording = Recording::from_bytes(&bytes)
                    .with_context(|| format!("{} is not a recording", path.display()))?;
                tracing::info!(
                    ticks = recording.ticks,
                    commands = recording.commands.len(),
                    digests = recording.digests.len(),
                    "replaying {}",
                    path.display()
                );
                Ok(Replay {
                    recording,
                    next: 0,
                    checked: 0,
                    diverged: None,
                })
            })
            .transpose()?
            .map(Box::new);
        Mode::Local {
            world: Box::new(world),
            recording: Recording::new(),
            replay,
        }
    };
    Ok((
        scene,
        Lab {
            mode,
            queued: Vec::new(),
            previous: current.clone(),
            current,
            pending: 0.0,
            tick_ms: Vec::new(),
            run_tick_ms: Vec::new(),
            awake: 0,
            logged: [60, 300, 600, 900, 1200],
            record: args.record.clone(),
        },
    ))
}

impl Lab {
    /// The world the screen shows: the local one, or the client's prediction.
    fn shown(&mut self) -> &mut LabWorld {
        match &mut self.mode {
            Mode::Local { world, .. } => world,
            Mode::Net(net) => &mut net.client.sim,
        }
    }

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
            self.tick();
            let ms = start.elapsed().as_secs_f64() * 1e3;
            self.tick_ms.push(ms);
            self.run_tick_ms.push(ms);
            std::mem::swap(&mut self.previous, &mut self.current);
            let mut current = std::mem::take(&mut self.current);
            let shown = self.shown();
            shown.transforms(&mut current);
            let (now, awake) = (shown.now(), shown.awake());
            self.current = current;
            self.awake = awake;
            if self.logged.contains(&now) {
                let digest = self.shown().digest();
                // The boat, when there is one: where it is (x, y, z) and how fast it goes.
                let player = self.shown().player().map_or(String::from("none"), |p| {
                    let p = p.position;
                    format!("{:.2},{:.2},{:.2}", p.x, p.y, p.z)
                });
                // What the scene rides (the boat, the car, the aeroplane): where it is and how
                // fast it goes.
                let (ride, ride_speed) = {
                    let shown = self.shown();
                    shown.ride().map_or((String::from("none"), 0.0), |k| {
                        let body = shown.bodies[k];
                        let (mut t, mut v) = (Vec::new(), Vec::new());
                        shown.world.transforms(&[body], &mut t);
                        shown.world.velocities(&[body], &mut v);
                        let p = t[0].position;
                        (
                            format!("{:.1},{:.2},{:.1}", p.x, p.y, p.z),
                            v[0].linear.length(),
                        )
                    })
                };
                // The wall's mortar: joints still holding, of all; the mannequins still on their
                // stands; the dominoes down, of all.
                let (mortar, stands, fallen) = {
                    let shown = self.shown();
                    (
                        shown.wall.as_ref().map_or(String::from("none"), |w| {
                            format!("{}/{}", w.holding(&shown.world), w.mortar.len())
                        }),
                        shown.herd.as_ref().map_or(String::from("none"), |h| {
                            h.standing(&shown.world).to_string()
                        }),
                        shown.run.as_ref().map_or(String::from("none"), |r| {
                            format!("{}/{}", r.fallen(&shown.world), r.dominoes.len())
                        }),
                    )
                };
                tracing::info!(
                    tick = now,
                    digest = format!("{digest:#018x}"),
                    awake,
                    ride,
                    ride_speed = format!("{ride_speed:.2}"),
                    player,
                    mortar,
                    stands,
                    fallen,
                    "physics lab state"
                );
            }
        }
    }

    /// One tick: the commands made since the last, or the replay's, then the step.
    fn tick(&mut self) {
        match &mut self.mode {
            Mode::Local {
                world,
                recording,
                replay,
            } => {
                let now = world.now();
                let mut these: Vec<Stamped<LabCommand>> = match replay {
                    Some(r) => {
                        let first = r.next;
                        while r.next < r.recording.commands.len()
                            && r.recording.commands[r.next].tick == now
                        {
                            r.next += 1;
                        }
                        r.recording.commands[first..r.next].to_vec()
                    }
                    None => self
                        .queued
                        .drain(..)
                        .enumerate()
                        .map(|(k, command)| Stamped {
                            tick: now,
                            player: 0,
                            seq: (recording.commands.len() + k) as u32,
                            command,
                        })
                        .collect(),
                };
                self.queued.clear();
                forge_sim::order(&mut these);
                world.tick(&these);
                recording.commands.extend(these);
                recording.ticks += 1;
                if world.now().is_multiple_of(RECORD_EVERY) {
                    let digest = world.digest();
                    recording.digests.push((world.now(), digest));
                    if let Some(r) = replay
                        && let Some(&(_, expected)) =
                            r.recording.digests.iter().find(|&&(t, _)| t == world.now())
                    {
                        r.checked += 1;
                        if expected != digest && r.diverged.is_none() {
                            tracing::warn!(
                                tick = world.now(),
                                expected = format!("{expected:#018x}"),
                                got = format!("{digest:#018x}"),
                                "the replay left the recording"
                            );
                            r.diverged = Some(world.now());
                        }
                    }
                }
            }
            Mode::Net(net) => net.tick(&mut self.queued),
        }
    }

    /// The movers' transforms, between the last two ticks by the time not yet ticked.
    pub(crate) fn movers(&self) -> Vec<MoverTransform> {
        let t = (self.pending / TICK).clamp(0.0, 1.0);
        self.previous
            .iter()
            .zip(&self.current)
            .map(|(a, b)| {
                // A body moved far in one tick (a ball thrown from its parking place) is drawn
                // where it is, not swept there.
                let far = a.position.distance_squared(b.position) > 100.0;
                MoverTransform {
                    position: if far {
                        b.position.as_vec3()
                    } else {
                        a.position.lerp(b.position, f64::from(t)).as_vec3()
                    },
                    rotation: a.rotation.slerp(b.rotation, t),
                    scale: 1.0,
                }
            })
            .collect()
    }

    /// Throws a ball from `from` along `forward` at the next tick.
    pub(crate) fn throw(&mut self, from: Vec3, forward: Vec3) {
        self.queued.push(LabCommand::Throw {
            from: from.to_array(),
            forward: forward.normalize_or(Vec3::NEG_Z).to_array(),
        });
    }

    /// Everything back to the start at the next tick.
    pub(crate) fn reset(&mut self) {
        self.queued.push(LabCommand::Reset);
    }

    /// The boat's motor from the next tick: throttle and rudder, −1 to 1.
    pub(crate) fn steer(&mut self, throttle: f32, rudder: f32) {
        self.queued.push(LabCommand::Steer { throttle, rudder });
    }

    /// The sea's time as drawn (seconds), when the scene has the sea: the time of the state
    /// between the last two ticks the movers show.
    pub(crate) fn sea_time(&mut self) -> Option<f64> {
        let t = f64::from((self.pending / TICK).clamp(0.0, 1.0));
        let shown = self.shown();
        shown
            .has_sea()
            .then(|| (shown.now() as f64 - 1.0 + t).max(0.0) * f64::from(TICK))
    }

    /// The player's walk from the next tick: m/s along the ground, world x and z.
    pub(crate) fn walk(&mut self, velocity: [f32; 2]) {
        self.queued.push(LabCommand::Walk { velocity });
    }

    /// The player jumps at the next tick, if on firm ground.
    pub(crate) fn jump(&mut self) {
        self.queued.push(LabCommand::Jump);
    }

    /// Whether the scene has a player to walk.
    pub(crate) fn has_player(&mut self) -> bool {
        self.shown().player().is_some()
    }

    /// The player as drawn, when the scene has one: its capsule's mover (the last but one).
    pub(crate) fn player(&mut self) -> Option<MoverTransform> {
        self.shown().player()?;
        let movers = self.movers();
        movers.get(movers.len().checked_sub(2)?).copied()
    }

    /// What the camera follows (C) as drawn: the boat or the car, when the scene has one.
    pub(crate) fn ride(&mut self) -> Option<MoverTransform> {
        let k = self.shown().ride()?;
        self.movers().get(k).copied()
    }

    /// Whether the scene has a car.
    pub(crate) fn has_car(&mut self) -> bool {
        self.shown().driver.car.is_some()
    }

    /// The car's handbrake from the next tick.
    pub(crate) fn handbrake(&mut self, on: bool) {
        self.queued.push(LabCommand::Handbrake { on });
    }

    /// Whether the scene has an aeroplane.
    pub(crate) fn has_plane(&mut self) -> bool {
        self.shown().pilot.plane.is_some()
    }

    /// The aeroplane's controls from the next tick: throttle, elevator, ailerons, rudder.
    pub(crate) fn fly(&mut self, controls: [f32; 4]) {
        self.queued.push(LabCommand::Fly { controls });
    }

    /// Whether the scene holds something back for Space to let go: the wrecking ball, the
    /// flood's gate, the dominoes all standing.
    pub(crate) fn held(&mut self) -> bool {
        let shown = self.shown();
        shown.wall.as_ref().is_some_and(|w| w.held(&shown.world))
            || shown
                .dam
                .as_ref()
                .zip(shown.water.as_ref())
                .is_some_and(|(dam, water)| dam.closed(water))
            || shown
                .run
                .as_ref()
                .is_some_and(|run| run.fallen(&shown.world) == 0)
    }

    /// The flood's water as the shown world holds it, for its drawing.
    pub(crate) fn pool(&mut self) -> Option<PoolView> {
        let water = self.shown().water.as_ref()?;
        Some(PoolView {
            origin: [water.origin[0] as f32, water.origin[1] as f32],
            spacing: water.spacing,
            size: [water.size[0] as u32, water.size[1] as u32],
            samples: flood::samples(water),
        })
    }

    /// Lets the wrecking ball go at the next tick.
    pub(crate) fn release(&mut self) {
        self.queued.push(LabCommand::Release);
    }

    /// Whether the scene has creatures, and whether their motors are let go.
    pub(crate) fn creatures(&mut self) -> Option<bool> {
        let shown = self.shown();
        shown.herd.as_ref().map(|_| shown.limp)
    }

    /// The creatures' motors let go (or powered again) from the next tick.
    pub(crate) fn limp(&mut self, on: bool) {
        self.queued.push(LabCommand::Limp { on });
    }

    /// The title's part: the ticks' time since the last title, the bodies awake, the session.
    pub(crate) fn title(&mut self) -> String {
        let n = self.tick_ms.len().max(1) as f64;
        let mean = self.tick_ms.iter().sum::<f64>() / n;
        let max = self.tick_ms.iter().copied().fold(0.0, f64::max);
        self.tick_ms.clear();
        let (bodies, now) = {
            let shown = self.shown();
            (shown.bodies(), shown.now())
        };
        let title = format!(
            "physics {mean:.2} ms a tick (max {max:.2}), {} of {bodies} awake, tick {now}",
            self.awake
        );
        match &mut self.mode {
            Mode::Net(net) => format!("{title} | {}", net.title()),
            Mode::Local {
                replay: Some(r), ..
            } => format!(
                "{title} | replay: {} digests checked, {}",
                r.checked,
                r.diverged
                    .map_or("all the recording's".to_owned(), |t| format!(
                        "left it at tick {t}"
                    ))
            ),
            Mode::Local { .. } => title,
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let mut ticks = std::mem::take(&mut self.run_tick_ms);
        if ticks.is_empty() {
            return;
        }
        let mean = ticks.iter().sum::<f64>() / ticks.len() as f64;
        let digest = self.shown().digest();
        tracing::info!(
            ticks = ticks.len(),
            mean = format!("{mean:.3}"),
            p99 = format!("{:.3}", super::percentile(&mut ticks, 0.99)),
            max = format!("{:.3}", super::percentile(&mut ticks, 1.0)),
            digest = format!("{digest:#018x}"),
            awake = self.awake,
            "physics ticks (ms) over the run"
        );
        match &self.mode {
            Mode::Local {
                recording, replay, ..
            } => {
                if let Some(r) = replay {
                    tracing::info!(
                        checked = r.checked,
                        diverged = ?r.diverged,
                        "the replay against its recording"
                    );
                }
                if let Some(path) = &self.record {
                    match std::fs::write(path, recording.to_bytes()) {
                        Ok(()) => tracing::info!(
                            ticks = recording.ticks,
                            commands = recording.commands.len(),
                            digests = recording.digests.len(),
                            "recorded {}",
                            path.display()
                        ),
                        Err(e) => tracing::warn!("writing {}: {e}", path.display()),
                    }
                }
            }
            Mode::Net(net) => {
                let (c, s) = (net.client.stats, net.server.stats);
                tracing::info!(
                    snapshots = c.snapshots,
                    matched = c.matched,
                    corrected = c.corrected,
                    replayed_ticks = c.replayed_ticks,
                    worst_correction_ms = format!("{:.2}", net.worst_correction_ms),
                    server_applied = s.applied,
                    server_late = s.late,
                    up_bytes = net.up[0].stats.bytes,
                    down_bytes = net.down[0].stats.bytes,
                    lost = net.up[0].stats.lost + net.down[0].stats.lost,
                    "the session over the run (this player's client)"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests' workers: one pool for them all.
    fn test_pool() -> Arc<TaskPool> {
        static POOL: std::sync::OnceLock<Arc<TaskPool>> = std::sync::OnceLock::new();
        Arc::clone(POOL.get_or_init(|| Arc::new(TaskPool::client())))
    }

    fn throws(n: u32, every: u64) -> Vec<Stamped<LabCommand>> {
        (0..n)
            .map(|k| Stamped {
                tick: 30 + u64::from(k) * every,
                player: 0,
                seq: k,
                command: LabCommand::Throw {
                    from: [8.0 + k as f32, 3.0, 9.0],
                    forward: Vec3::new(-0.6, 0.1, -0.8).normalize().to_array(),
                },
            })
            .collect()
    }

    #[test]
    fn a_command_survives_its_bytes() {
        let steer = LabCommand::Steer {
            throttle: -0.4,
            rudder: 1.0,
        };
        let walk = LabCommand::Walk {
            velocity: [-1.5, 2.25],
        };
        for c in [
            throws(1, 1)[0].command,
            LabCommand::Reset,
            steer,
            walk,
            LabCommand::Jump,
            LabCommand::Handbrake { on: true },
            LabCommand::Fly {
                controls: [0.5, -1.0, 0.25, 0.0],
            },
            LabCommand::Release,
        ] {
            let mut bytes = Vec::new();
            c.encode(&mut bytes);
            let mut read = bytes.as_slice();
            assert_eq!(LabCommand::decode(&mut read), Some(c));
            assert!(read.is_empty());
        }
    }

    #[test]
    fn the_lab_replays_a_recording_to_the_same_digests() {
        let (mut first, _) = LabWorld::new(LabScene::Drop, test_pool()).unwrap();
        let mut commands = throws(6, 40);
        commands.push(Stamped {
            tick: 200,
            player: 0,
            seq: 6,
            command: LabCommand::Reset,
        });
        let recording = Recording::record(&mut first, commands, 360, 60);
        let (mut second, _) = LabWorld::new(LabScene::Drop, test_pool()).unwrap();
        recording.replay(&mut second).expect("the same digests");
        // And a world that saw one throw fewer leaves it.
        let mut fewer = recording.clone();
        fewer.commands.remove(2);
        let (mut third, _) = LabWorld::new(LabScene::Drop, test_pool()).unwrap();
        assert!(fewer.replay(&mut third).is_err());
    }

    #[test]
    fn the_sea_replays_a_recording_waves_boat_and_all() {
        // A throw, the boat ahead and turning, then easing off: 4 s of the sea scene.
        let steer = |tick: u64, seq: u32, throttle: f32, rudder: f32| Stamped {
            tick,
            player: 0,
            seq,
            command: LabCommand::Steer { throttle, rudder },
        };
        let mut commands = throws(1, 1);
        commands.push(steer(40, 1, 1.0, 0.5));
        commands.push(steer(180, 2, 0.3, -1.0));
        let (mut first, _) = LabWorld::new(LabScene::Sea, test_pool()).unwrap();
        let start = first.boat().unwrap();
        let recording = Recording::record(&mut first, commands, 240, 60);
        // The boat went somewhere under its motor.
        let moved = first.boat().unwrap().position.distance(start.position);
        assert!(moved > 3.0, "the boat moved {moved} m");
        let (mut second, _) = LabWorld::new(LabScene::Sea, test_pool()).unwrap();
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_playground_replays_a_walk_up_the_stairs_and_a_jump() {
        let command = |tick: u64, seq: u32, command: LabCommand| Stamped {
            tick,
            player: 0,
            seq,
            command,
        };
        let commands = vec![
            command(
                20,
                0,
                LabCommand::Walk {
                    velocity: [2.0, 0.0],
                },
            ),
            command(150, 1, LabCommand::Jump),
            command(
                200,
                2,
                LabCommand::Walk {
                    velocity: [0.0, 2.0],
                },
            ),
        ];
        let (mut first, _) = LabWorld::new(LabScene::Walk, test_pool()).unwrap();
        let recording = Recording::record(&mut first, commands, 300, 60);
        // Up the stairs and over them, then along +z.
        let end = first.player().unwrap().position;
        assert!(end.x > 4.0 && end.z > 2.0, "the player ended at {end}");
        let (mut second, _) = LabWorld::new(LabScene::Walk, test_pool()).unwrap();
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_track_replays_a_drive_with_a_turn_and_the_handbrake() {
        let command = |tick: u64, seq: u32, command: LabCommand| Stamped {
            tick,
            player: 0,
            seq,
            command,
        };
        let steer = |throttle: f32, rudder: f32| LabCommand::Steer { throttle, rudder };
        let commands = vec![
            command(30, 0, steer(1.0, 0.0)),
            command(150, 1, steer(1.0, 0.4)),
            command(240, 2, LabCommand::Handbrake { on: true }),
            command(270, 3, LabCommand::Handbrake { on: false }),
            command(280, 4, steer(-1.0, 0.0)),
        ];
        let (mut first, _) = LabWorld::new(LabScene::Drive, test_pool()).unwrap();
        let car = first.driver.car.unwrap().0;
        let mut t = Vec::new();
        first.world.transforms(&[car], &mut t);
        let start = t[0].position;
        let recording = Recording::record(&mut first, commands, 330, 60);
        first.world.transforms(&[car], &mut t);
        let moved = t[0].position.distance(start);
        assert!(moved > 10.0, "the car moved {moved} m");
        assert!((t[0].rotation * Vec3::Y).y > 0.9, "upright");
        let (mut second, _) = LabWorld::new(LabScene::Drive, test_pool()).unwrap();
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_field_replays_a_take_off() {
        let fly = |tick: u64, seq: u32, controls: [f32; 4]| Stamped {
            tick,
            player: 0,
            seq,
            command: LabCommand::Fly { controls },
        };
        // Full throttle and the stick a little back: it lifts off near 30 m/s, 20 s on. Then a
        // roll to the right with some rudder, the wings held banked, the stick further back.
        let commands = vec![
            fly(10, 0, [1.0, -0.4, 0.0, 0.0]),
            fly(1200, 1, [1.0, -0.5, 0.5, 0.2]),
            fly(1260, 2, [1.0, -0.6, 0.0, 0.0]),
        ];
        let (mut first, _) = LabWorld::new(LabScene::Fly, test_pool()).unwrap();
        let plane = first.pilot.plane.unwrap();
        let mut t = Vec::new();
        first.world.transforms(&[plane], &mut t);
        let start = t[0].position;
        let recording = Recording::record(&mut first, commands, 1500, 120);
        first.world.transforms(&[plane], &mut t);
        let (end, rotation) = (t[0].position, t[0].rotation);
        // Climbing (35 m at the time of writing), turned right (by 48°), banked 35°.
        assert!(end.y > start.y + 20.0, "airborne: {end}");
        assert!(end.x > 20.0, "turned right: {end}");
        assert!((rotation * Vec3::Y).y > 0.6, "upright");
        let (mut second, _) = LabWorld::new(LabScene::Fly, test_pool()).unwrap();
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_wall_stands_until_the_ball_breaks_it_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Break, test_pool()).unwrap();
        let wall = first.wall.clone().unwrap();
        let all = wall.mortar.len();
        let mut built = Vec::new();
        first.world.transforms(&first.bodies, &mut built);
        // Two seconds untouched: no mortar breaks, the wall settles by a centimetre at most
        // (Jolt's contacts give a little; 24 courses of it), stays upright and goes to sleep.
        for _ in 0..120 {
            first.tick(&[]);
        }
        assert_eq!(wall.holding(&first.world), all);
        let mut t = Vec::new();
        first.world.transforms(&first.bodies, &mut t);
        let (settled, leant) = built.iter().zip(&t).fold((0.0, 0.0), |(s, l), (a, b)| {
            let d = b.position - a.position;
            (f64::max(s, d.length()), f64::max(l, d.z.abs()))
        });
        assert!(
            settled < 0.015 && leant < 0.002,
            "settled {settled} m, leant {leant} m"
        );
        assert!(first.awake() < 10, "{} bodies awake", first.awake());
        // Let go: the ball swings into the wall, breaks its mortar and knocks bricks out.
        let commands = vec![Stamped {
            tick: 120,
            player: 0,
            seq: 0,
            command: LabCommand::Release,
        }];
        let recording = Recording::record(&mut first, commands, 240, 60);
        let broken = all - wall.holding(&first.world);
        first.world.transforms(&first.bodies, &mut t);
        let knocked = built
            .iter()
            .zip(&t)
            .filter(|(a, b)| a.position.distance(b.position) > 0.5)
            .count();
        assert!(
            broken > 50 && knocked > 20,
            "{broken} joints of {all} broken, {knocked} bricks knocked out"
        );
        // The same two seconds from a fresh world, to the same digests.
        let (mut second, _) = LabWorld::new(LabScene::Break, test_pool()).unwrap();
        for _ in 0..120 {
            second.tick(&[]);
        }
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_creatures_hold_their_poses_go_limp_and_replay() {
        let (mut first, _) = LabWorld::new(LabScene::Creatures, test_pool()).unwrap();
        let herd = first.herd.clone().unwrap();
        // Two seconds on their motors: the mannequins on their stands, the dogs on their feet.
        for _ in 0..120 {
            first.tick(&[]);
        }
        assert_eq!(herd.standing(&first.world), 3);
        let dogs = |world: &World| -> Vec<f64> {
            herd.roots(world)
                .into_iter()
                .filter(|(kind, _)| *kind == creatures::Kind::Dog)
                .map(|(_, t)| t.position.y)
                .collect()
        };
        assert!(
            dogs(&first.world).iter().all(|&y| y > 0.45),
            "{:?}",
            dogs(&first.world)
        );
        // Let go: the dogs fold to the ground, the mannequins hang on their stands.
        let commands = vec![Stamped {
            tick: 120,
            player: 0,
            seq: 0,
            command: LabCommand::Limp { on: true },
        }];
        let recording = Recording::record(&mut first, commands, 180, 60);
        assert!(
            dogs(&first.world).iter().all(|&y| y < 0.35),
            "{:?}",
            dogs(&first.world)
        );
        assert_eq!(herd.standing(&first.world), 3);
        let (mut second, _) = LabWorld::new(LabScene::Creatures, test_pool()).unwrap();
        for _ in 0..120 {
            second.tick(&[]);
        }
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_flood_waits_for_its_gate_then_carries_what_floats_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Flood, test_pool()).unwrap();
        let start = first.water.as_ref().unwrap().volume();
        let downstream = |world: &LabWorld| {
            let w = world.water.as_ref().unwrap();
            let gate =
                ((f64::from(flood::GATE_X) + 1.0 - w.origin[0]) / f64::from(w.spacing)) as usize;
            let area = f64::from(w.spacing * w.spacing);
            (0..w.size[1])
                .flat_map(|z| (gate..w.size[0]).map(move |x| (x, z)))
                .map(|(x, z)| f64::from(w.depth[w.index(x, z)]) * area)
                .sum::<f64>()
        };
        let reach = |world: &LabWorld| {
            let bodies: Vec<BodyId> = world.floaters.iter().map(|f| f.body).collect();
            let mut t = Vec::new();
            world.world.transforms(&bodies, &mut t);
            t.iter().map(|t| t.position.x).fold(f64::MIN, f64::max)
        };
        // A second with the gate shut: the reservoir stays behind it.
        for _ in 0..60 {
            first.tick(&[]);
        }
        assert!(downstream(&first) < 1e-6 * start, "{}", downstream(&first));
        let before = reach(&first);
        // Lifted: in four seconds the water runs down the basin, every drop kept, and carries
        // what floats past where it lay.
        let commands = vec![Stamped {
            tick: 60,
            player: 0,
            seq: 0,
            command: LabCommand::Release,
        }];
        let recording = Recording::record(&mut first, commands, 240, 60);
        let water = first.water.as_ref().unwrap();
        assert!(
            (water.volume() - start).abs() < 1e-3 * start,
            "{} of {start}",
            water.volume()
        );
        assert!(
            downstream(&first) > 0.3 * start,
            "{} of {start}",
            downstream(&first)
        );
        assert!(
            reach(&first) > before + 5.0,
            "{} from {before}",
            reach(&first)
        );
        let (mut second, _) = LabWorld::new(LabScene::Flood, test_pool()).unwrap();
        for _ in 0..60 {
            second.tick(&[]);
        }
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_dominoes_fall_to_the_last_and_replay() {
        let (mut first, _) = LabWorld::new(LabScene::Dominoes, test_pool()).unwrap();
        let run = first.run.clone().unwrap();
        // Standing a second untouched: none falls.
        for _ in 0..60 {
            first.tick(&[]);
        }
        assert_eq!(run.fallen(&first.world), 0);
        // Pushed: the fall runs the spiral out, every domino down within 50 s (the wave runs at
        // about 2.5 m/s, all down at tick 2 940 at the time of writing).
        let commands = vec![Stamped {
            tick: 60,
            player: 0,
            seq: 0,
            command: LabCommand::Release,
        }];
        let recording = Recording::record(&mut first, commands, 3000, 120);
        let fallen = run.fallen(&first.world);
        assert_eq!(fallen, run.dominoes.len(), "{fallen} down");
        let (mut second, _) = LabWorld::new(LabScene::Dominoes, test_pool()).unwrap();
        for _ in 0..60 {
            second.tick(&[]);
        }
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn a_session_over_a_lossy_link_ends_where_the_server_is() {
        let mut net = Net::new(LabScene::Drop, 100.0, &test_pool()).unwrap();
        let mine = throws(5, 50);
        let mut queued = Vec::new();
        // The bot throws three times (its ticks 150, 300, 450), then everyone runs on quiet.
        for k in 0..720 {
            net.bot_on = k < 480;
            let now = net.client.sim.now();
            queued.extend(mine.iter().filter(|c| c.tick == now).map(|c| c.command));
            net.tick(&mut queued);
        }
        assert_eq!(net.server.stats.applied, 5 + 3);
        assert_eq!(net.server.stats.late, 0);
        let (c, b) = (net.client.stats, net.bot.stats);
        assert!(c.matched > 0 && c.corrected > 0, "{c:?}");
        assert!(
            b.corrected > 0,
            "the bot learns of the player's throws: {b:?}"
        );
        // The client, a lead ahead, against the server run on to its tick with nothing more.
        let state = net.server.sim.save();
        let (mut server, _) = LabWorld::new(LabScene::Drop, test_pool()).unwrap();
        server.restore(&state);
        while server.now() < net.client.sim.now() {
            server.tick(&[]);
        }
        assert_eq!(server.digest(), net.client.sim.digest());
    }
}

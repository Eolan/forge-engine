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
use forge_app::Setup;
use forge_geom::SkinnedMesh;
use forge_geom::city::{Block, Lathe, PropKind, PropSpec};
use forge_physics::buoyancy::{Fluid, Hull};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, Velocity, World, WorldDesc};
use forge_render::material::ModelTextures;
use forge_render::meshlet::MeshId;
use forge_render::{MeshletScene, MeshletSceneBuilder, MoverTransform};
use forge_sim::{
    Client, Codec, InputPacket, Link, LinkParams, PlayerId, Recording, Server, Simulation,
    Snapshot, Stamped, TICK, TICK_RATE,
};
use forge_task::TaskPool;
use glam::{DVec3, Mat4, Quat, Vec2, Vec3};

use super::{Args, CityMaterials, Cooked, barrel_prop, scene_origin};

mod bonds;
mod bridge;
mod creatures;
pub(crate) use creatures::ArmPose;
mod dominoes;
mod drive;
mod flood;
mod fly;
mod flyer;
pub(crate) mod materials;
pub(crate) use materials::ROWS as MATERIAL_ROWS;
pub(crate) mod models;
mod rocket;
pub(crate) mod room;
mod sea;
mod ship;
mod slime;
pub(crate) use slime::FLAVOURS as SLIME_FLAVOURS;
mod space;
pub(crate) mod tank;
mod tug;
pub(crate) mod walk;
mod wall;
pub(crate) mod yard;

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
    /// The dogs alone on a course: three steps of 5 cm up and down, a ramp of 10° up and down
    /// (#167's feet on uneven ground).
    Course,
    /// The dogs walking their lanes over a bed of damp sand and a bed of fresh snow, pressing
    /// their prints into them (#185: the deformable ground, D-007).
    Yard,
    /// Gulls circling over the floor on their wings' lift, beating and gliding (#184).
    Flyer,
    /// A dam break: a reservoir behind a gate, a basin with blocks and a hut, what floats (#144).
    Flood,
    /// A domino run on a spiral (#146).
    Dominoes,
    /// A bridge collapsing under a convoy of cars (#147).
    Bridge,
    /// A rocket on a launch pad (#148).
    Rocket,
    /// A tug-of-war on a sled, two teams pulling (#149); with `--net`, against the bot.
    Tug,
    /// A sci-fi spaceship in zero g over a planet under the stars, crates floating ahead of it
    /// (#150).
    Space,
    /// A dam break in a glass tank on a table: the GPU liquid, 640 000 particles on a 1 cm grid
    /// (#156, D-044).
    Tank,
    /// The same tank as a bench to tune the liquid by: no glass, frame or table to see, a floor of
    /// 10 cm squares, a plain background, the sun alone, the water tinted (#156).
    TankBench,
    /// The glass tank with the gate fixed and a round hole through it, a shutter over it: the jet
    /// (#156).
    TankHole,
    /// The glass tank's dam break with concrete blocks in the water's way: a cube, two posts
    /// (#156).
    TankBlocks,
    /// A plain room to measure sharpness by: white walls, a floor of black and white squares, black
    /// squares turned 5° on the back wall and on a board, the sun alone (#159).
    Room,
    /// Patches of brick, wood, sand, snow and ice along the walker's way, a crate on a ramp and
    /// a falling ball on each, every body's friction and restitution from its material's row
    /// (#203, D-007).
    Materials,
    /// Models made by others, the Khronos glTF sample assets fetched by `tools/fetch-assets.sh`
    /// (#170, D-048): each on a plinth, or one alone with `--model`.
    Models,
}

/// The floor's half side, metres.
const FLOOR_HALF: f32 = 400.0;
/// The drawn floor's half depth, metres (its physics stays a slab a metre thick): deep enough that
/// the light probes under it, lit by the void's sky and meeting its underside, stand out of reach
/// of the points above it. A metre down, they leaked that light into everything within 4 m of
/// the floor: in Sponza a glow along the curtains' hems (#171).
const FLOOR_DRAWN_HALF: f32 = 10.0;
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
/// The mannequins' pole (the creatures are skinned meshes, #165).
const POLE: usize = PIECE + wall::PIECES;
/// The group of bodies the skinned creatures draw ([`Skinned`]), not a prop.
const SKINNED: usize = usize::MAX;
/// The slimes' bodies (`slime`), drawn skinned after the creatures, not a prop.
const SLIME: usize = usize::MAX - 1;
/// The gulls' bodies (`flyer`), drawn skinned after the slimes, not a prop.
const BIRD: usize = usize::MAX - 2;
/// The flood's walls along x and z, its gate, a block, the hut.
const FLOOD: usize = POLE + 1;
/// The dominoes' prop, after the flood's five.
const DOMINO: usize = FLOOD + 5;
/// The bridge's bank and deck panel, after the domino.
const BRIDGE: usize = DOMINO + 1;
/// The rocket, its fins and its pad, after the bridge's two.
const ROCKET: usize = BRIDGE + 2;
/// The tug-of-war's rope and line, after the rocket's three.
const TUG: usize = ROCKET + 3;
/// The spaceship and its engines' flame, after the tug-of-war's two.
const SHIP: usize = TUG + 2;
/// The glass tank's table, its frame's bars along x, y and z, its gate, its bench's four floors,
/// its holed gate and the shutter, the blocks' cube and post, after the ship's two.
const TANK: usize = SHIP + 2;
/// The sharpness room's floor, back wall, side wall, target, board and the board's target, after the
/// tank's thirteen.
const ROOM: usize = TANK + 13;
/// The dogs' course's step, ramp and landing (#167), after the room's six.
const COURSE: usize = ROOM + 6;
/// A dog's paw print (#167's foot-down events), after the course's three.
const PRINT: usize = COURSE + 3;
/// The materials' patches, a prop per row (#203), after the print.
const MATERIALS: usize = PRINT + 1;
/// The models scene's plinth, then its models' meshes, after the patches (only in that scene).
const MODELS: usize = MATERIALS + materials::ROWS.len();
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
const WORDS: usize = 17;
/// Ticks between the digests a recording keeps.
const RECORD_EVERY: u64 = 60;
/// `--net`: ticks between the server's snapshots, the share of packets lost, ticks between the
/// bot's throws.
const SNAPSHOT_EVERY: u64 = 6;
const NET_LOSS: f64 = 0.02;
const BOT_EVERY: u64 = 150;
/// `--net` in the tug-of-war: ticks between the bot's changes of pull.
const BOT_PULL_EVERY: u64 = 90;

/// The props the lab draws, in this order: the floor, the block, the barrel, the rocks, the
/// ball, then the sea's: the crate, the log, the pillar, the deck, the boat; then the
/// playground's: the stairs' slab, the ramp, the platform, the player and its visor; then the
/// car's body and wheel; then the aeroplane, its propeller, the runway and the field.
pub(crate) fn props(scene: LabScene) -> Vec<PropSpec> {
    let mut props = drop_props();
    props.extend(sea::props());
    props.extend(walk::props());
    props.extend(drive::props());
    props.extend(fly::props());
    props.extend(wall::props());
    props.extend(creatures::props());
    props.extend(flood::props());
    props.extend(dominoes::props());
    props.extend(bridge::props());
    props.extend(rocket::props());
    props.extend(tug::props());
    props.extend(ship::props());
    props.extend(tank::props());
    props.extend(room::props());
    props.extend(creatures::course_props());
    props.extend(materials::props());
    // Only their scene reads and cooks the external models.
    if scene == LabScene::Models {
        props.extend(models::props());
    }
    props
}

/// The pyramid's props.
fn drop_props() -> Vec<PropSpec> {
    let mut props = vec![
        PropSpec {
            name: "lab-floor".to_owned(),
            kind: PropKind::Block(Block {
                half: [FLOOR_HALF, FLOOR_DRAWN_HALF, FLOOR_HALF],
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
    /// What the scene holds back let go: the wrecking ball (#142), the flood's gate (#144), the
    /// first domino (#146), the convoy (#147).
    Release,
    /// The creatures' motors let go, or powered again (#143), held until the next.
    Limp {
        /// Let go.
        on: bool,
    },
    /// The player's team's pull in the tug-of-war (#149), held until the next.
    Pull {
        /// 0 to 1 of full strength.
        strength: f32,
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
            Self::Pull { strength } => {
                out.push(9);
                out.extend_from_slice(&strength.to_bits().to_le_bytes());
            }
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
            9 => {
                let (word, rest) = bytes.split_first_chunk::<4>()?;
                *bytes = rest;
                Some(Self::Pull {
                    strength: f32::from_bits(u32::from_le_bytes(*word)),
                })
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
    /// The tick of the last reset (0 before any): the yard's autopilot times its stop in the
    /// sand from it (#195).
    started: u64,
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
    /// Their dogs' paws coming down (#167's foot-down events).
    feet: creatures::Feet,
    /// The slimes before the creatures, soft bodies (#179, #180).
    slimes: Vec<slime::Slime>,
    /// The gulls (#184).
    birds: Vec<flyer::Bird>,
    /// The yard's beds of sand and snow, which the dogs' footfalls press (#185).
    beds: Vec<yard::Bed>,
    /// How many times the beds changed: the renderer sends their heights up when it moves (not
    /// part of the state).
    beds_changed: u64,
    /// The car's beds as the physics stands on them (#187).
    grounds: Vec<yard::Ground>,
    /// The flood's water (the authoritative column model) and its dam.
    water: Option<forge_physics::shallow::Pool>,
    dam: Option<flood::Dam>,
    water_start: Option<forge_physics::shallow::Pool>,
    /// The domino run, and the bridge with its convoy.
    run: Option<dominoes::Run>,
    convoy: Option<bridge::Convoy>,
    /// The rocket, flown with the pilot's controls.
    rocket: Option<BodyId>,
    /// The tug-of-war's sled, and the teams' pulls (0 to 1): the left for player 0, the right
    /// for player 1.
    tug: Option<BodyId>,
    pulls: [f32; 2],
    /// The spaceship, flown with the pilot's controls in space: no gravity, no air (#150).
    ship: Option<BodyId>,
    /// The glass tank's gate (#156).
    tank: Option<tank::Tank>,
    limp: bool,
    platform: Option<BodyId>,
    /// The materials' patches, the rows their bodies took, and what is measured (#203).
    materials: Option<materials::Yard>,
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
            // In space, none (#150).
            gravity: if kind == LabScene::Space {
                Vec3::ZERO
            } else {
                WorldDesc::default().gravity
            },
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
            | LabScene::Course
            | LabScene::Yard
            | LabScene::Flyer
            | LabScene::Flood
            | LabScene::Dominoes
            | LabScene::Bridge
            | LabScene::Rocket
            | LabScene::Tug
            | LabScene::Space
            | LabScene::Tank
            | LabScene::TankBench
            | LabScene::TankHole
            | LabScene::TankBlocks
            | LabScene::Room
            | LabScene::Materials
            | LabScene::Models => 0.0,
        };
        // The flight's is a field of grass, wide enough to fly over for a while.
        let (floor, floor_half, drawn_half) = match kind {
            LabScene::Fly | LabScene::Rocket => (FIELD, fly::FIELD_HALF, 0.5),
            _ => (FLOOR, FLOOR_HALF, FLOOR_DRAWN_HALF),
        };
        let floor_at = Vec3::new(0.0, floor_y - 0.5, 0.0);
        // None in space: the ship flies over a planet far below; none on the tank's bench nor in the
        // sharpness room, whose floors are their own.
        let mut statics = Vec::new();
        if !matches!(kind, LabScene::Space | LabScene::TankBench | LabScene::Room) {
            let floor_shape = Shape::cuboid(Vec3::new(floor_half, 0.5, floor_half), 0.05, 0.0)?;
            world.add_body(&BodyDesc::fixed(&floor_shape, floor_at.as_dvec3()))?;
            statics.push((
                floor,
                Mat4::from_translation(Vec3::new(0.0, floor_y - drawn_half, 0.0)),
            ));
        }
        // The shapes, each with its origin where its mesh has its own: the barrel's and the
        // ball's at their bottom, the rocks' hulls from their meshes' vertices.
        let block_shape = Shape::cuboid(Vec3::splat(BLOCK_HALF), 0.03, 2300.0)?;
        let half_length = super::BARREL_LENGTH * 0.5;
        let up = |h: f32| Vec3::new(0.0, h, 0.0);
        let barrel_shape = Shape::cylinder(half_length, super::BARREL_RADIUS, 0.03, 300.0)?
            .offset(up(half_length), Quat::IDENTITY)?;
        let ball_shape =
            Shape::sphere(BALL_RADIUS, 500.0)?.offset(up(BALL_RADIUS), Quat::IDENTITY)?;
        let props = props(kind);
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
        let mut yard = None;
        let mut driver = drive::Driver::default();
        let mut pilot = fly::Pilot::default();
        let mut wall = None;
        let mut herd = None;
        let mut slimes = Vec::new();
        let mut birds = Vec::new();
        let mut beds = Vec::new();
        let mut grounds = Vec::new();
        let mut water = None;
        let mut dam = None;
        let mut run = None;
        let mut convoy = None;
        let mut rocket = None;
        let mut ship = None;
        let mut tank = None;
        let mut tug = None;
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
            LabScene::Materials => {
                let crate_shape = Shape::cuboid(Vec3::splat(sea::CRATE_HALF), 0.025, 150.0)?;
                let field = materials::build(
                    &mut world,
                    MATERIALS,
                    &crate_shape,
                    sea::CRATE_HALF,
                    &ball_shape,
                )?;
                statics.extend(field.statics);
                group(CRATE, field.crates, &mut bodies);
                group(BALL, field.balls, &mut bodies);
                player.character = Some(field.player);
                yard = Some(field.yard);
                // The beds the walker's boots press (#205), drawn as the yard's are: through
                // the skinned group, which no body of the scene's is in.
                beds = materials::beds();
                group(SKINNED, Vec::new(), &mut bodies);
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
                let field = creatures::build(&mut world, POLE, COURSE, creatures::Ground::Flat)?;
                statics.extend(field.statics);
                group(SKINNED, field.bodies, &mut bodies);
                herd = Some(field.herd);
                slimes = slime::build(&mut world)?;
                group(SLIME, slimes.iter().map(|s| s.body).collect(), &mut bodies);
            }
            LabScene::Course => {
                let field = creatures::build(&mut world, POLE, COURSE, creatures::Ground::Course)?;
                statics.extend(field.statics);
                group(SKINNED, field.bodies, &mut bodies);
                herd = Some(field.herd);
            }
            LabScene::Yard => {
                let field = creatures::build(&mut world, POLE, COURSE, creatures::Ground::Yard)?;
                statics.extend(field.statics);
                group(SKINNED, field.bodies, &mut bodies);
                herd = Some(field.herd);
                beds = yard::beds();
                // The car's beds as ground (#187).
                grounds = yard::grounds(&mut world, &beds)?;
                // The car beside the dogs, on its own beds (#186).
                // Its wheels finding its soft ground with rays (#189).
                let car = drive::car_on(
                    &mut world,
                    DVec3::new(yard::LANE, 0.15, yard::CAR_START),
                    true,
                )?;
                group(CAR, vec![car.0], &mut bodies);
                driver.car = Some(car);
            }
            LabScene::Flyer => {
                // Drawn skinned after no creature.
                group(SKINNED, Vec::new(), &mut bodies);
                birds = flyer::build(&mut world)?;
                group(BIRD, birds.iter().map(|b| b.body).collect(), &mut bodies);
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
            LabScene::Bridge => {
                let site = bridge::build(&mut world, BRIDGE)?;
                statics.extend(site.statics);
                group(BRIDGE + 1, site.panels, &mut bodies);
                let cars = site.convoy.cars.iter().map(|c| c.0).collect();
                group(CAR, cars, &mut bodies);
                convoy = Some(site.convoy);
            }
            LabScene::Rocket => {
                let site = rocket::build(&mut world, ROCKET + 2)?;
                statics.extend(site.statics);
                group(ROCKET, vec![site.rocket], &mut bodies);
                rocket = Some(site.rocket);
            }
            LabScene::Tug => {
                let site = tug::build(&mut world, TUG + 1)?;
                statics.extend(site.statics);
                group(CRATE, vec![site.sled], &mut bodies);
                tug = Some(site.sled);
            }
            LabScene::Space => {
                let crate_shape = Shape::cuboid(Vec3::splat(sea::CRATE_HALF), 0.025, 150.0)?;
                let site = space::build(&mut world, &crate_shape)?;
                group(SHIP, vec![site.ship], &mut bodies);
                group(CRATE, site.crates, &mut bodies);
                ship = Some(site.ship);
            }
            LabScene::Tank | LabScene::TankBench | LabScene::TankHole | LabScene::TankBlocks => {
                let built = tank::build(
                    &mut world,
                    TANK,
                    kind == LabScene::TankBench,
                    kind == LabScene::TankHole,
                    kind == LabScene::TankBlocks,
                )?;
                statics.extend(built.statics);
                group(built.mover.0, vec![built.mover.1], &mut bodies);
                tank = Some(built.tank);
            }
            LabScene::Room => statics.extend(room::build(ROOM)),
            LabScene::Models => statics.extend(models::build(MODELS)),
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
        // The rocket's two pairs of fins.
        if rocket.is_some() {
            groups.push(Group {
                prop: ROCKET + 1,
                count: 2,
            });
        }
        // The ship's engines' flames.
        if ship.is_some() {
            groups.push(Group {
                prop: SHIP + 1,
                count: ship::FLAMES,
            });
        }
        // The tug-of-war's two ropes.
        if tug.is_some() {
            groups.push(Group {
                prop: TUG,
                count: 2,
            });
        }
        // The car's four wheels after the bodies' movers.
        if driver.car.is_some() {
            groups.push(Group {
                prop: WHEEL,
                count: 4,
            });
        }
        // The convoy's, car by car.
        if let Some(convoy) = &convoy {
            groups.push(Group {
                prop: WHEEL,
                count: 4 * convoy.cars.len() as u32,
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
        // The dogs' paw prints after everything.
        if herd.is_some() {
            groups.push(Group {
                prop: PRINT,
                count: creatures::PRINTS as u32,
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
                started: 0,
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
                feet: creatures::Feet::default(),
                slimes,
                birds,
                beds,
                grounds,
                beds_changed: 0,
                water_start: water.clone(),
                water,
                dam,
                run,
                convoy,
                rocket,
                pulls: tug.map_or([0.0; 2], |_| [tug::HOLD; 2]),
                tug,
                ship,
                tank,
                limp: false,
                materials: yard,
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
        if let Some(convoy) = &self.convoy {
            convoy.wheels(&self.world, out);
        }
        self.pilot.propeller(&self.world, self.tick, out);
        if let Some(r) = self.rocket {
            rocket::fins_at(&self.world, r, out);
        }
        if let Some(s) = self.ship {
            ship::flames_at(&self.world, s, self.pilot.throttle, out);
        }
        if let Some(sled) = self.tug {
            tug::ropes(&self.world, sled, out);
        }
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
        // The dogs' paw prints last.
        if self.herd.is_some() {
            creatures::prints(&self.feet, &yard::SOFT, DVec3::new(0.0, PARKED_Y, 0.0), out);
        }
    }

    /// The slimes' points in the world after the last step into `out`, one slime's after the
    /// other's (none without them).
    pub(crate) fn slime_points(&self, out: &mut Vec<Vec3>) {
        out.clear();
        let mut one = Vec::new();
        for s in &self.slimes {
            s.points(&self.world, &mut one);
            out.extend_from_slice(&one);
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
    /// creatures let go, the tug-of-war's two pulls (0 where there is none).
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
            self.pulls[0],
            self.pulls[1],
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

    /// The mover of what a player rides, the boat, the car, the aeroplane or the rocket: its body's
    /// place among the bodies, which the movers draw first.
    pub(crate) fn ride(&self) -> Option<usize> {
        let body = self
            .boat
            .map(|b| b.body)
            .or(self.driver.car.map(|c| c.0))
            .or(self.pilot.plane)
            .or(self.rocket)
            .or(self.ship)
            .or(self.birds.first().map(|b| b.body))?;
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
                    self.started = self.tick;
                    self.feet.clear();
                    if let Some(yard) = &mut self.materials {
                        yard.stride = crate::island_walk::Stride::default();
                    }
                    if !self.beds.is_empty() {
                        self.beds = if self.materials.is_some() {
                            materials::beds()
                        } else {
                            yard::beds()
                        };
                        self.beds_changed += 1;
                        yard::settle(&mut self.world, &mut self.beds, &mut self.grounds, true);
                    }
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
                    if self.tug.is_some() {
                        self.pulls = [tug::HOLD; 2];
                    }
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
                    if let Some(convoy) = &self.convoy {
                        convoy.release(&mut self.world);
                    }
                    if let Some(tank) = &self.tank {
                        tank.open(&mut self.world);
                    }
                }
                LabCommand::Limp { on } => self.limp = on,
                LabCommand::Pull { strength } => {
                    if self.tug.is_some()
                        && let Some(pull) = self.pulls.get_mut(usize::from(c.player))
                    {
                        *pull = strength.clamp(0.0, 1.0);
                    }
                }
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
        // The flood: what floats pushed by its water as it stands, the water pushed aside by
        // what floats (#151), then the water a tick on.
        if let Some(water) = &mut self.water {
            let volumes = sea::float(
                &mut self.world,
                &self.floaters,
                &self.hulls,
                &*water,
                &FRESH,
                &self.pool,
            );
            flood::displace(water, &self.world, &self.floaters, &self.hulls, &volumes);
            water.step(TICK);
        }
        // The materials' walker on its autopilot (#203), and where its feet were (#205).
        let mut feet_before = None;
        if let (Some(yard), Some(c)) = (&self.materials, self.player.character) {
            let position = self.world.character(c).position;
            self.player.walk = yard.walk(position.z);
            feet_before = Some(position);
        }
        // The playground's platform and player, before the bodies move: on the materials'
        // patches its feet grip as their rows do (#203).
        let yard = self.materials.as_ref();
        self.player
            .tick(&mut self.world, self.platform, self.tick, TICK, |body| {
                yard.and_then(|y| y.row(body))
                    .map(|row| materials::traction(&row.physics, TICK))
            });
        // Its boots pressing the sand and the snow as its stride brings them down (#205).
        if let (Some(yard), Some(c), Some(from)) =
            (&mut self.materials, self.player.character, feet_before)
        {
            let s = self.world.character(c);
            if s.ground == forge_physics::Ground::Firm
                && let Some(fall) = yard.stride.step(from, s.position, f64::from(TICK))
                && materials::press(&mut self.beds, fall)
            {
                self.beds_changed += 1;
            }
        }
        // The yard's car on its autopilot unless the player drives it (#186).
        if self.beds.is_empty() {
            self.driver.tick(&mut self.world);
        } else {
            yard::drive(
                &mut self.world,
                &self.driver,
                &self.beds,
                self.tick - self.started,
            );
        }
        if let Some(convoy) = &self.convoy {
            convoy.tick(&mut self.world);
        }
        self.pilot.tick(&mut self.world);
        if let Some(r) = self.rocket {
            rocket::tick(&mut self.world, r, &self.pilot);
        }
        if let Some(s) = self.ship {
            ship::tick(&mut self.world, s, &self.pilot);
        }
        if let Some(sled) = self.tug {
            tug::pull(&mut self.world, sled, self.pulls);
        }
        let column = self.wall.as_ref().map(|w| w.column_velocity(&self.world));
        // The creatures' motors driven to their poses at this tick (or let go), the yard's dogs'
        // paws on their beds (#194).
        if let Some(herd) = &self.herd {
            let beds = &self.beds;
            herd.drive(
                &mut self.world,
                self.tick as f64 * f64::from(TICK),
                self.limp,
                &|x, z| yard::top(beds, x, z),
            );
        }
        slime::drive_all(&mut self.slimes, &mut self.world, self.tick);
        flyer::drive(&mut self.world, &mut self.birds, self.tick);
        if let Err(e) = self.world.step(TICK, 1) {
            tracing::warn!("physics tick {}: {e}", self.tick);
        }
        // A mannequin knocked hard enough comes off its stand.
        if let Some(herd) = &self.herd {
            herd.knock(&mut self.world, TICK);
            // Its dogs' paws that came down in the step.
            let beds = &self.beds;
            let came = herd.feel(&self.world, self.tick, &mut self.feet, &|x, z| {
                yard::top(beds, x, z)
            });
            // Those on the yard's beds pressed into them.
            if yard::press(&mut self.beds, self.feet.last_mut(came)) > 0 {
                self.beds_changed += 1;
            }
        }
        // The yard's car's wheels on its beds press their patches in (#186).
        if let (false, Some(car)) = (self.beds.is_empty(), self.driver.car)
            && yard::roll(&mut self.beds, &self.world, car, TICK) > 0
        {
            self.beds_changed += 1;
            // What its wheels pressed, under them for the next step (#187).
            yard::settle(&mut self.world, &mut self.beds, &mut self.grounds, false);
        }
        // The lifted gate stops at its top.
        if let Some(dam) = &self.dam {
            dam.tick(&mut self.world);
        }
        if let Some(tank) = &self.tank {
            tank.tick(&mut self.world);
        }
        // The mortar that carried more than it holds in that step breaks, and the column
        // shatters under a blow.
        if let (Some(wall), Some(before)) = (&self.wall, column) {
            wall.crack(&mut self.world, TICK);
            wall.shatter(&mut self.world, before, TICK);
        }
        // The deck's joints that carried more than they hold break.
        if let Some(convoy) = &self.convoy {
            convoy.deck.crack(&mut self.world, TICK);
        }
        // The materials' crates, balls and walker, measured against their rows (#203).
        if let Some(yard) = &mut self.materials {
            yard.watch(&self.world, self.tick + 1);
        }
        self.tick += 1;
    }

    fn now(&self) -> u64 {
        self.tick
    }

    fn save(&mut self) -> Vec<u8> {
        let mut out = self.tick.to_le_bytes().to_vec();
        out.extend_from_slice(&self.started.to_le_bytes());
        out.extend_from_slice(&self.next_throw.to_le_bytes());
        // The dogs' paws down and their footfalls' count.
        let (down, falls) = self.feet.state();
        out.extend_from_slice(&down.to_le_bytes());
        out.extend_from_slice(&falls.to_le_bytes());
        // The gulls' clips.
        for bird in &self.birds {
            for word in bird.state() {
                out.extend_from_slice(&word.to_le_bytes());
            }
        }
        // The yard's beds' heights, then their treads' relief (#188).
        for bed in &self.beds {
            for h in bed.layer.heights().iter().chain(bed.layer.relief()) {
                out.extend_from_slice(&h.to_bits().to_le_bytes());
            }
        }
        // The materials' walker's stride (#205).
        if let Some(yard) = &self.materials {
            out.extend_from_slice(&yard.stride.walked.to_bits().to_le_bytes());
            out.push(u8::from(yard.stride.left));
        }
        // The car's grounds as Jolt holds them, a millimetre from their beds at most (#191).
        for ground in &self.grounds {
            for s in ground.samples() {
                out.extend_from_slice(&s.to_bits().to_le_bytes());
            }
        }
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
        let (started, rest) = rest.split_at(8);
        let (next, rest) = rest.split_at(4);
        let (feet, rest) = rest.split_at(16);
        let (flock, mut rest) = rest.split_at(32 * self.birds.len());
        let mut values = Vec::new();
        for bed in &mut self.beds {
            let [nx, nz] = bed.layer.size();
            for relief in [false, true] {
                let (bytes, after) = rest.split_at(4 * (nx * nz) as usize);
                values.clear();
                values.extend(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|&b| f32::from_bits(u32::from_le_bytes(b))),
                );
                if relief {
                    bed.layer.set_relief(&values);
                } else {
                    bed.layer.set_heights(&values);
                }
                rest = after;
            }
        }
        self.beds_changed += 1;
        if let Some(yard) = &mut self.materials {
            let (walked, after) = rest.split_at(8);
            let (left, after) = after.split_at(1);
            yard.stride = crate::island_walk::Stride {
                walked: f64::from_bits(u64::from_le_bytes(walked.try_into().expect("8 bytes"))),
                left: left[0] != 0,
            };
            rest = after;
        }
        // The car's grounds as they were saved, not their beds: Jolt's saved state holds no
        // shape (#187), and the grounds lag their beds (#191).
        let rest = yard::set_grounds(&mut self.world, &mut self.beds, &mut self.grounds, rest);
        let (words, world) = rest.split_at(4 * WORDS);
        self.tick = u64::from_le_bytes(tick.try_into().expect("8 bytes"));
        self.started = u64::from_le_bytes(started.try_into().expect("8 bytes"));
        self.next_throw = u32::from_le_bytes(next.try_into().expect("4 bytes"));
        let (down, falls) = feet.split_at(8);
        self.feet.set_state((
            u64::from_le_bytes(down.try_into().expect("8 bytes")),
            u64::from_le_bytes(falls.try_into().expect("8 bytes")),
        ));
        for (bird, words) in self.birds.iter_mut().zip(flock.as_chunks::<32>().0) {
            let (words, _) = words.as_chunks::<8>();
            bird.set_state(std::array::from_fn(|k| u64::from_le_bytes(words[k])));
        }
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
        self.pulls = [w[15], w[16]];
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
            ^ self.started.rotate_left(41)
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
        // The yard's beds, every height and its relief to the bit (#185, #188).
        for (b, bed) in self.beds.iter().enumerate() {
            let mut h = 0xcbf2_9ce4_8422_2325_u64 ^ b as u64;
            for height in bed.layer.heights().iter().chain(bed.layer.relief()) {
                h = (h ^ u64::from(height.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
            }
            d ^= h.rotate_left(31);
        }
        // The materials' walker's stride (#205).
        if let Some(yard) = &self.materials {
            d ^= yard.stride.walked.to_bits().rotate_left(41) ^ u64::from(yard.stride.left);
        }
        // The car's grounds as Jolt holds them (#191).
        for (g, ground) in self.grounds.iter().enumerate() {
            let mut h = 0x8422_2325_cbf2_9ce4_u64 ^ g as u64;
            for s in ground.samples() {
                h = (h ^ u64::from(s.to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
            }
            d ^= h.rotate_left(37);
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
    /// Whether the bot plays, and whether it pulls (the tug-of-war) rather than throws.
    bot_on: bool,
    bot_pulls: bool,
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
            bot_pulls: kind == LabScene::Tug,
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
        // In the tug-of-war the bot (the right team) pulls hard and eases off in turn, every
        // 1.5 s; elsewhere it throws at the pyramid from one of eight places round it, every 2.5 s.
        let bot_tick = self.bot.sim.now();
        if self.bot_pulls {
            if self.bot_on && bot_tick > 0 && bot_tick.is_multiple_of(BOT_PULL_EVERY) {
                let hard = (bot_tick / BOT_PULL_EVERY) % 2 == 1;
                self.bot.command(LabCommand::Pull {
                    strength: if hard { 1.0 } else { 0.3 },
                });
            }
        } else if self.bot_on && bot_tick > 0 && bot_tick.is_multiple_of(BOT_EVERY) {
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
    /// `--arm-pose` (#169): the mannequins' left forearm drawn twisted or bent on top of their
    /// pose, to compare the skinning's blends.
    pub(crate) arm_pose: Option<creatures::ArmPose>,
    /// `--elbow-correctives` (#169): the mannequins carry their elbows' corrective morph
    /// targets, weighted by how far each elbow bends.
    elbow_correctives: bool,
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
    /// The glass tank's liquid substeps the ticks since the last frame owe it (#156).
    tank_steps: Vec<forge_render::LiquidStep>,
    /// The skinned creatures (#165), in the creatures scene.
    skinned: Option<Skinned>,
    /// The slime's points in the world at the last two ticks, when there is one.
    slime_points: [Vec<Vec3>; 2],
}

/// Where the skinned creatures' bodies lie in the movers' transforms (from `start`, each
/// creature's parts in a row, the slime's body after them), and each one's kind (#165).
struct Skinned {
    start: usize,
    kinds: Vec<creatures::Kind>,
    /// How many slimes' bodies follow theirs.
    slimes: usize,
    /// How many gulls' bodies follow the slimes'.
    birds: usize,
    /// Where the yard's beds are drawn, after the gulls (#185): no body's, a mover each.
    beds: Vec<Vec3>,
}

/// The sea scene's water (#138): the island's cascades of FFT waves from the same seed as the
/// physics' heights, and an open surface with no shore.
pub(crate) fn water(
    ctx: &Setup,
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
    ctx: &Setup,
    args: &Args,
    cooked: Cooked,
    kind: LabScene,
) -> Result<(MeshletScene, Lab)> {
    let start = Instant::now();
    let props = props(kind);
    let mut builder = MeshletSceneBuilder::new();
    let ids: Vec<MeshId> = cooked.meshes.iter().map(|m| builder.add_mesh(m)).collect();
    let mut materials = CityMaterials::new(&ctx.device)?;
    // Each model's rows, one per material of its mesh in order (its sections), as the file
    // gives them: the boat, the car's body and its wheel, the aeroplane and its propeller, the
    // spaceship and its flame (#138, #140, #141; the ship's glowing parts emissive).
    for (model, label, meshes) in [
        (sea::boat_model(), "boat", &[("lab-boat", "boat")][..]),
        (
            drive::car_model(),
            "car",
            &[("lab-car", "car"), ("lab-wheel", "car-wheel")],
        ),
        (
            fly::plane_model(),
            "plane",
            &[("lab-plane", "plane"), ("lab-propeller", "plane-prop")],
        ),
        (
            ship::ship_model(),
            "ship",
            &[("lab-ship", "ship"), ("lab-ship-flame", "ship-flame")],
        ),
    ] {
        let mut textures = ModelTextures::new(&model.0, label);
        for &(prop, mesh) in meshes {
            let mesh = model
                .0
                .mesh(mesh)
                .with_context(|| format!("the {label} model's {mesh}"))?;
            model_rows(&mut materials, &mut textures, prop, mesh, true);
        }
    }
    // The creatures' (#143), their wood and fur painted on their UVs (#166, D-047): the
    // textures stay on the bending bodies. Only their scene decodes the images.
    let (creatures_model, _) = creatures::model();
    let mut textures = ModelTextures::new(creatures_model, "skinned-creatures");
    for (prop, mesh) in [("lab-mannequin", "mannequin-body"), ("lab-dog", "dog-body")] {
        let mesh = creatures_model
            .mesh(mesh)
            .with_context(|| format!("the creatures' model's {mesh}"))?;
        model_rows(
            &mut materials,
            &mut textures,
            prop,
            mesh,
            matches!(
                kind,
                LabScene::Creatures | LabScene::Course | LabScene::Yard
            ),
        );
    }
    // The external models' (#170, D-048), in their scene only.
    if kind == LabScene::Models {
        for e in models::externals() {
            let mut textures = ModelTextures::new(&e.model, &e.name);
            for (k, mesh) in e.model.meshes.iter().enumerate() {
                model_rows(&mut materials, &mut textures, e.props[k], mesh, true);
            }
            // A room is walked inside: its shadows and bounced light need its walls where they
            // are drawn. Cut to the default 40 000 triangles, Sponza's 262 000 stood up to
            // 0.7 m off, which left a dark band over a vault and let the probes see gaps (#171).
            if e.is_room() {
                let meshes: Vec<MeshId> = e
                    .props
                    .iter()
                    .filter_map(|name| props.iter().position(|p| p.name == *name))
                    .map(|i| ids[i])
                    .collect();
                builder.set_ray_group(&meshes, models::ROOM_RAY_BUDGET);
            }
        }
    }
    // The gulls' (#184), their feathers baked on their UVs, in their scene only.
    if kind == LabScene::Flyer {
        let (bird_model, _) = flyer::model();
        let mut textures = ModelTextures::new(bird_model, "bird");
        let mesh = bird_model
            .mesh("bird-body")
            .context("the bird's model's body")?;
        model_rows(&mut materials, &mut textures, "lab-bird", mesh, true);
    }
    let creature_rows = [materials.of("lab-mannequin"), materials.of("lab-dog")];
    // The beds' rows: the yard's soft materials, and the materials scene's sand and snow (#205).
    let soft_rows: Vec<_> = yard::SOFT
        .iter()
        .chain(&[MATERIAL_ROWS[2].name, MATERIAL_ROWS[3].name])
        .map(|&name| (name, materials.of(name)))
        .collect();
    let bird_row = materials.of("lab-bird");
    let slime_rows = SLIME_FLAVOURS.map(|(name, _)| materials.of(name));
    materials.apply(&mut builder, &props, &ids);
    builder.set_ray_traced(!args.no_shadows);
    builder.set_origin(scene_origin(args));
    let pool = Arc::new(TaskPool::client());
    let (world, layout) = LabWorld::new(kind, Arc::clone(&pool))?;
    for &(prop, at) in &layout.statics {
        builder.add_instance(ids[prop], at);
    }
    let mut per_mesh: Vec<(MeshId, u32)> = layout
        .groups
        .iter()
        .filter(|g| g.prop != SKINNED && g.prop != SLIME && g.prop != BIRD)
        .map(|g| (ids[g.prop], g.count))
        .collect();
    // The skinned creatures (#165): a mesh each (its vertices are its own), cooked once a
    // kind, bounded by the sphere about its root every pose stays in; their movers last.
    // `--dual-quaternion` skins them and the gulls by dual quaternions (#169).
    let skin_blend = if args.dual_quaternion {
        forge_render::SkinBlend::DualQuaternion
    } else {
        forge_render::SkinBlend::Linear
    };
    let skinned = layout
        .groups
        .iter()
        .position(|g| g.prop == SKINNED)
        .map(|g| {
            let start = layout.groups[..g].iter().map(|g| g.count as usize).sum();
            let kinds = world.herd.as_ref().map(|h| h.kinds()).unwrap_or_default();
            // `--elbow-correctives`: the mannequin's elbows' morph targets (#169).
            let kinds_cooked = [creatures::Kind::Mannequin, creatures::Kind::Dog].map(|kind| {
                let body = kind.body();
                let targets = if args.elbow_correctives && kind == creatures::Kind::Mannequin {
                    creatures::elbow_correctives()
                } else {
                    Vec::new()
                };
                SkinnedMesh::cook_morphed(&body.mesh, &body.skin, &targets, ([0.0; 3], body.reach))
            });
            for &kind in &kinds {
                let (k, row) = match kind {
                    creatures::Kind::Mannequin => (0, creature_rows[0]),
                    creatures::Kind::Dog => (1, creature_rows[1]),
                };
                let id = builder.add_skinned_mesh(&kinds_cooked[k], kind.body().joints as u32);
                builder.set_mesh_material(id, row);
                builder.set_skin_blend(id, skin_blend);
                per_mesh.push((id, 1));
            }
            // The slimes after them, one mesh each in its flavour: their points are its joints.
            let slimes = layout
                .groups
                .get(g + 1)
                .filter(|g| g.prop == SLIME)
                .map_or(0, |g| g.count as usize);
            if slimes > 0 {
                let s = slime::surface();
                // `--slime-eight` (#169): the drop on its eight nearest points a vertex.
                let cooked = if args.slime_eight {
                    SkinnedMesh::cook_full(
                        &s.mesh,
                        &s.eight,
                        &s.more,
                        &[],
                        ([0.0; 3], slime::BOUND),
                    )
                } else {
                    SkinnedMesh::cook(&s.mesh, &s.skin, ([0.0; 3], slime::BOUND))
                };
                for k in 0..slimes {
                    let id = builder.add_skinned_mesh(&cooked, slime::POINTS as u32);
                    builder.set_mesh_material(id, slime_rows[k % slime_rows.len()]);
                    per_mesh.push((id, 1));
                }
            }
            // The gulls after them, one mesh each.
            let birds = layout
                .groups
                .iter()
                .find(|g| g.prop == BIRD)
                .map_or(0, |g| g.count as usize);
            if birds > 0 {
                let (bird_model, rig) = flyer::model();
                let mesh = bird_model.mesh("bird-body").expect("the bird's body");
                let skin = mesh.skin.as_ref().expect("a skinned bird");
                // A model's second joint set when it has one (#169).
                let more = mesh.skin_more.as_deref().unwrap_or_default();
                let cooked =
                    SkinnedMesh::cook_full(&mesh.mesh, skin, more, &[], ([0.0; 3], flyer::REACH));
                for _ in 0..birds {
                    let id = builder.add_skinned_mesh(&cooked, rig.skeleton.len() as u32);
                    builder.set_mesh_material(id, bird_row);
                    builder.set_skin_blend(id, skin_blend);
                    per_mesh.push((id, 1));
                }
            }
            // The yard's beds after them (#185): a ground each that its layer's heights raise,
            // placed by its mover, its one joint unturned.
            let beds: Vec<Vec3> = world
                .beds
                .iter()
                .map(|bed| {
                    let cooked = SkinnedMesh::cook_displaced(&yard::ground(bed), yard::REACH);
                    let id = builder.add_displaced_mesh(
                        &cooked,
                        forge_render::HeightField {
                            origin: [0.0; 2],
                            cell: bed.layer.cell(),
                            size: bed.layer.size(),
                            slopes: false,
                        },
                    );
                    let row = soft_rows
                        .iter()
                        .find(|(name, _)| *name == bed.name)
                        .map(|&(_, row)| row)
                        .expect("a row for each bed's material");
                    builder.set_mesh_material(id, row);
                    per_mesh.push((id, 1));
                    yard::place(bed)
                })
                .collect();
            Skinned {
                start,
                kinds,
                slimes,
                birds,
                beds,
            }
        });
    builder.reserve_movers(&per_mesh);
    let mut scene = builder.build(&ctx.device)?;
    scene.build_tlas(&ctx.device, &ctx.shaders)?;

    let mut current = Vec::new();
    world.transforms(&mut current);
    let mut points = Vec::new();
    world.slime_points(&mut points);
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
            bot = if kind == LabScene::Tug {
                "pulls"
            } else {
                "throws"
            },
            "the lab through a server and a client, a bot playing too"
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
            arm_pose: None,
            elbow_correctives: args.elbow_correctives,
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
            tank_steps: Vec::new(),
            skinned,
            slime_points: [points.clone(), points],
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

    /// [`Lab::shown`], to read.
    fn seen(&self) -> &LabWorld {
        match &self.mode {
            Mode::Local { world, .. } => world,
            Mode::Net(net) => &net.client.sim,
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
            let gate_from = self.tank_state();
            self.tick();
            if let (Some(from), Some(to)) = (gate_from, self.tank_state()) {
                self.tank_steps.extend(tank::steps(from, to));
            }
            let ms = start.elapsed().as_secs_f64() * 1e3;
            self.tick_ms.push(ms);
            self.run_tick_ms.push(ms);
            std::mem::swap(&mut self.previous, &mut self.current);
            let mut current = std::mem::take(&mut self.current);
            let shown = self.shown();
            shown.transforms(&mut current);
            let (now, awake) = (shown.now(), shown.awake());
            self.current = current;
            self.slime_points.swap(0, 1);
            let mut points = std::mem::take(&mut self.slime_points[1]);
            self.shown().slime_points(&mut points);
            self.slime_points[1] = points;
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
                            format!("{}/{}", w.holding(&shown.world), w.mortar.joints.len())
                        }),
                        shown.herd.as_ref().map_or(String::from("none"), |h| {
                            h.standing(&shown.world).to_string()
                        }),
                        shown.run.as_ref().map_or(String::from("none"), |r| {
                            format!("{}/{}", r.fallen(&shown.world), r.dominoes.len())
                        }),
                    )
                };
                // The tug-of-war: where the sled is, the teams' pulls, who won.
                let tug = {
                    let shown = self.shown();
                    shown.tug.map_or(String::from("none"), |sled| {
                        let mut t = Vec::new();
                        shown.world.transforms(&[sled], &mut t);
                        let won = match tug::winner(&shown.world, sled) {
                            Some(0) => "the left",
                            Some(_) => "the right",
                            None => "nobody yet",
                        };
                        format!(
                            "x {:.2}, pulls {:.1} and {:.1}, won by {won}",
                            t[0].position.x, shown.pulls[0], shown.pulls[1]
                        )
                    })
                };
                // In space: the ship's and the crates' momentum, which should keep.
                let momentum = {
                    let shown = self.shown();
                    shown.ship.map_or(String::from("none"), |ship| {
                        let p =
                            space::momentum(&shown.world, ship, &shown.bodies[1..=space::CRATES]);
                        format!("{:.2},{:.2},{:.2}", p.x, p.y, p.z)
                    })
                };
                // The bridge's deck: joints still holding, of all; cars across and down.
                let deck = {
                    let shown = self.shown();
                    shown.convoy.as_ref().map_or(String::from("none"), |c| {
                        format!(
                            "{}/{}, {} across, {} down",
                            c.deck.holding(&shown.world),
                            c.deck.joints.len(),
                            c.across(&shown.world),
                            c.down(&shown.world)
                        )
                    })
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
                    deck,
                    tug,
                    momentum,
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

    /// The movers' transforms, between the last two ticks by the time not yet ticked, and the
    /// skinned creatures' joints' matrices into `skins` (#165): their bodies' transforms become
    /// a mover each, after the others, and the matrices that bend their meshes. With
    /// `--elbow-correctives`, the mannequins' morph weights into `morphs` (#169).
    pub(crate) fn movers(
        &self,
        skins: &mut Vec<Mat4>,
        morphs: &mut Vec<f32>,
    ) -> Vec<MoverTransform> {
        let t = (self.pending / TICK).clamp(0.0, 1.0);
        let mut movers: Vec<MoverTransform> = self
            .previous
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
            .collect();
        skins.clear();
        morphs.clear();
        if let Some(skinned) = &self.skinned {
            let parts = skinned.kinds.len() * creatures::PARTS;
            let end = skinned.start + parts + skinned.slimes + skinned.birds;
            let bodies: Vec<(Vec3, Quat)> = movers
                .drain(skinned.start..end)
                .map(|m| (m.position, m.rotation))
                .collect();
            let mut matrices = Vec::new();
            for (&kind, parts) in skinned
                .kinds
                .iter()
                .zip(bodies[..parts].chunks(creatures::PARTS))
            {
                let position = creatures::skin(kind, parts, &mut matrices);
                if let Some(pose) = self.arm_pose {
                    creatures::pose_arm(kind, pose, &mut matrices);
                }
                if self.elbow_correctives && kind == creatures::Kind::Mannequin {
                    morphs.extend(creatures::elbow_bends(&matrices));
                }
                skins.extend_from_slice(&matrices);
                movers.push(MoverTransform {
                    position,
                    rotation: Quat::IDENTITY,
                    scale: 1.0,
                });
            }
            // The slimes: their points between the last two ticks, about their bodies' places.
            let [was, now] = &self.slime_points;
            // Their looks between the last two ticks: the last tick ran as `now() - 1`.
            let seen = self.seen();
            let ticks = seen.now() as f64 - 1.0 + f64::from(t);
            for k in 0..skinned.slimes {
                let position = bodies[parts + k].0;
                let range = k * slime::POINTS..(k + 1) * slime::POINTS;
                let points: Vec<Vec3> = was[range.clone()]
                    .iter()
                    .zip(&now[range])
                    .map(|(a, b)| a.lerp(*b, t) - position)
                    .collect();
                slime::skin(&points, seen.slimes[k].look(ticks), &mut matrices);
                skins.extend_from_slice(&matrices);
                movers.push(MoverTransform {
                    position,
                    rotation: Quat::IDENTITY,
                    scale: 1.0,
                });
            }
            // The gulls: their clips' poses between the last two ticks, on their bodies.
            if skinned.birds > 0 {
                let mut pose = flyer::model().1.skeleton.rest().clone();
                for k in 0..skinned.birds {
                    let (position, rotation) = bodies[parts + skinned.slimes + k];
                    seen.birds[k].pose_at(ticks, &mut pose);
                    flyer::skin(&pose, rotation, &mut matrices);
                    skins.extend_from_slice(&matrices);
                    movers.push(MoverTransform {
                        position,
                        rotation: Quat::IDENTITY,
                        scale: 1.0,
                    });
                }
            }
            // The yard's beds: where they lie, their grounds' one joint unturned.
            for &position in &skinned.beds {
                skins.push(Mat4::IDENTITY);
                movers.push(MoverTransform {
                    position,
                    rotation: Quat::IDENTITY,
                    scale: 1.0,
                });
            }
        }
        movers
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

    /// The player as drawn, when the scene has one: its capsule's mover (the last but one
    /// before the beds', #205).
    pub(crate) fn player(&mut self) -> Option<MoverTransform> {
        self.shown().player()?;
        let beds = self.skinned.as_ref().map_or(0, |s| s.beds.len());
        let movers = self.movers(&mut Vec::new(), &mut Vec::new());
        movers.get(movers.len().checked_sub(2 + beds)?).copied()
    }

    /// What the camera follows (C) as drawn: the boat or the car, when the scene has one.
    pub(crate) fn ride(&mut self) -> Option<MoverTransform> {
        let k = self.shown().ride()?;
        // A gull is drawn skinned, its mover unturned: its body's own transform.
        if self.has_birds() {
            let t = (self.pending / TICK).clamp(0.0, 1.0);
            let (a, b) = (self.previous.get(k)?, self.current.get(k)?);
            return Some(MoverTransform {
                position: a.position.lerp(b.position, f64::from(t)).as_vec3(),
                rotation: a.rotation.slerp(b.rotation, t),
                scale: 1.0,
            });
        }
        self.movers(&mut Vec::new(), &mut Vec::new())
            .get(k)
            .copied()
    }

    /// Whether the scene has gulls (the camera follows the first).
    pub(crate) fn has_birds(&mut self) -> bool {
        !self.shown().birds.is_empty()
    }

    /// The yard's beds' heights after the last tick into `out` (#185), every bed's in turn, as
    /// their grounds' height fields take them, when they changed since `since` (a count this
    /// returns); `out` empty without beds or a change.
    pub(crate) fn fields(&self, since: u64, out: &mut Vec<f32>) -> u64 {
        let seen = self.seen();
        out.clear();
        if seen.beds_changed != since {
            yard::heights(&seen.beds, out);
        }
        seen.beds_changed
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

    /// Whether the scene is the tug-of-war.
    pub(crate) fn has_tug(&mut self) -> bool {
        self.shown().tug.is_some()
    }

    /// This player's team's pull in the tug-of-war from the next tick (0 to 1).
    pub(crate) fn pull(&mut self, strength: f32) {
        self.queued.push(LabCommand::Pull { strength });
    }

    /// Whether the scene has a rocket, flown with the aeroplane's controls.
    pub(crate) fn has_rocket(&mut self) -> bool {
        self.shown().rocket.is_some()
    }

    /// Whether the scene has the spaceship, flown with the aeroplane's controls (in space).
    pub(crate) fn has_ship(&mut self) -> bool {
        self.shown().ship.is_some()
    }

    /// The tank's gate and shutter bottoms over its floor, metres, in the shown world.
    fn tank_state(&mut self) -> Option<[f32; 2]> {
        let shown = self.shown();
        shown.tank.as_ref().map(|t| t.state(&shown.world))
    }

    /// The liquid's substeps the ticks since the last call owe it, with where the gate stood.
    pub(crate) fn take_tank_steps(&mut self) -> Vec<forge_render::LiquidStep> {
        std::mem::take(&mut self.tank_steps)
    }

    /// The shown world's tick.
    pub(crate) fn now(&mut self) -> u64 {
        self.shown().now()
    }

    /// The aeroplane's controls from the next tick: throttle, elevator, ailerons, rudder.
    pub(crate) fn fly(&mut self, controls: [f32; 4]) {
        self.queued.push(LabCommand::Fly { controls });
    }

    /// Whether the scene holds something back for Space to let go: the wrecking ball, the
    /// flood's gate, the dominoes all standing, the convoy, the tank's gate or shutter.
    pub(crate) fn held(&mut self) -> bool {
        let shown = self.shown();
        shown.wall.as_ref().is_some_and(|w| w.held(&shown.world))
            || shown.tank.as_ref().is_some_and(|t| t.closed(&shown.world))
            || shown
                .dam
                .as_ref()
                .zip(shown.water.as_ref())
                .is_some_and(|(dam, water)| dam.closed(water))
            || shown
                .run
                .as_ref()
                .is_some_and(|run| run.fallen(&shown.world) == 0)
            || shown.convoy.as_ref().is_some_and(|c| c.held(&shown.world))
    }

    /// The flood's columns as the shown world holds them, and the world's tick, for the GPU's
    /// layer that shadows them (#162).
    pub(crate) fn columns(&mut self) -> Option<(u64, &forge_physics::shallow::Pool)> {
        let shown = self.shown();
        let now = shown.now();
        shown.water.as_ref().map(|water| (now, water))
    }

    /// Where the flood's water splashes as the shown world holds it (#162), and the spray the
    /// yard's car's slipping wheels throw (#192), added to `out`.
    pub(crate) fn splashes(&mut self, out: &mut Vec<forge_render::SplashSource>) {
        let shown = self.shown();
        if let Some(water) = shown.water.as_ref() {
            flood::splashes(water, out);
        }
        if let (false, Some(car)) = (shown.beds.is_empty(), shown.driver.car) {
            yard::sprays(&shown.world, &shown.beds, car, out);
        }
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

/// The rows of a model's mesh `mesh` for `prop`, one per material (its sections), as the file
/// gives them, through the model's `textures`; with `textured` and UVs on the mesh, its maps too
/// (D-047).
fn model_rows(
    materials: &mut CityMaterials,
    textures: &mut ModelTextures,
    prop: &'static str,
    mesh: &forge_geom::model::ModelMesh,
    textured: bool,
) {
    let uvs = textured && !mesh.mesh.uvs.is_empty();
    let rows = mesh
        .materials
        .iter()
        .map(|m| {
            (
                m.name.clone(),
                textures.layer(m, uvs, &mut materials.textures),
            )
        })
        .collect();
    materials.add_rows(prop, rows);
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
    fn the_materials_rows_hold_and_slide_the_crates_and_the_walker_slides_on_ice() {
        // Along its way on the autopilot, stopping on the ice: 900 ticks see it rest.
        let (mut first, _) = LabWorld::new(LabScene::Materials, test_pool()).unwrap();
        let recording = Recording::record(&mut first, Vec::new(), 900, 60);
        let yard = first.materials.as_ref().unwrap();
        let (slid, predicted) = yard.slide().expect("the walker at rest on the ice");
        assert!(
            (slid - predicted).abs() < 0.05 * predicted,
            "slid {slid:.2} m against {predicted:.2}"
        );
        // The crates hold on brick, wood and sand (their pairs' friction over tan 20°), slide
        // off the snow's ramp and the ice's.
        let travel = yard.crate_travel(&first.world);
        assert!(travel[..3].iter().all(|&d| d < 0.02), "{travel:?}");
        assert!(travel[3] > 1.0 && travel[4] > 1.0, "{travel:?}");
        // Its boots' prints in the sand and the snow (#205): a few on each, along its way (the
        // feet 10 cm either side of it at x = 0), the sand's 1.5 cm deep at least, the snow's
        // 4 cm.
        let untouched = materials::beds();
        for ((bed, before), deepest) in first.beds.iter().zip(&untouched).zip([0.015_f32, 0.04]) {
            let [nx, _] = bed.layer.size();
            let (origin, cell) = (bed.layer.origin(), f64::from(bed.layer.cell()));
            // Against the bed untouched: its edges thin to nothing on their own.
            let pressed: Vec<f64> = bed
                .layer
                .heights()
                .iter()
                .zip(before.layer.heights())
                .enumerate()
                .filter(|&(_, (&h, &was))| h < was - deepest)
                .map(|(k, _)| origin.x + (k as u32 % nx) as f64 * cell)
                .collect();
            assert!(pressed.len() > 100, "{} points pressed", pressed.len());
            assert!(pressed.iter().all(|x| x.abs() < 0.3), "a print off the way");
        }
        // A grip changes no walk where the ground has no row: the playground's replay, above.
        let (mut second, _) = LabWorld::new(LabScene::Materials, test_pool()).unwrap();
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
        let all = wall.mortar.joints.len();
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
    fn the_yard_keeps_its_prints_through_a_save_clears_them_on_a_reset_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Yard, test_pool()).unwrap();
        for _ in 0..500 {
            first.tick(&[]);
        }
        // Saved and restored with the car spinning its wheels in its sand (#191) and the dogs on
        // their beds (#194): the same digest at once and a second on, its grounds as Jolt held
        // them (rebuilt from the beds, which they lag by up to a millimetre, it went its own way).
        let spinning = first.save();
        let (mut third, _) = LabWorld::new(LabScene::Yard, test_pool()).unwrap();
        third.restore(&spinning);
        assert_eq!(
            third.digest(),
            first.digest(),
            "at once from a save mid-spin"
        );
        for _ in 0..60 {
            first.tick(&[]);
            third.tick(&[]);
        }
        assert_eq!(third.digest(), first.digest(), "on from a save mid-spin");
        for _ in 0..340 {
            first.tick(&[]);
        }
        let untouched = yard::beds();
        assert!(
            first.beds.iter().zip(&untouched).all(|(a, b)| a != b),
            "prints in both beds"
        );
        // Saved and restored into a fresh yard: the same prints, the same digest.
        let state = first.save();
        let (mut second, _) = LabWorld::new(LabScene::Yard, test_pool()).unwrap();
        second.restore(&state);
        assert_eq!(second.beds, first.beds);
        assert_eq!(second.digest(), first.digest());
        // On for a while, then back to the start: the car's beds untouched again; the dogs',
        // where they stand at the start, holding the prints of the reset's own tick alone (#194);
        // and the same run replayed to the same digests from the restored yard.
        let printed = |beds: &[yard::Bed]| -> Vec<usize> {
            beds.iter()
                .zip(&untouched)
                .map(|(a, b)| {
                    let (a, b) = (a.layer.heights(), b.layer.heights());
                    a.iter().zip(b).filter(|(x, y)| x != y).count()
                })
                .collect()
        };
        let before = printed(&first.beds);
        let commands = vec![Stamped {
            tick: 1200,
            player: 0,
            seq: 0,
            command: LabCommand::Reset,
        }];
        let recording = Recording::record(&mut first, commands, 301, 60);
        assert_eq!(first.beds[3..], untouched[3..]);
        let after = printed(&first.beds);
        assert!(
            (0..3).all(|k| after[k] * 10 < before[k]),
            "{after:?} points printed after the reset, {before:?} before"
        );
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_yard_car_stops_in_its_sand_and_spins_out_again_after_a_reset() {
        let (mut lab, _) = LabWorld::new(LabScene::Yard, test_pool()).unwrap();
        // Where the car is along its lane, its pace, and how far its faster driven wheel's tread
        // runs past it (m/s).
        let car = |lab: &LabWorld| {
            let (chassis, vehicle) = lab.driver.car.expect("the yard's car");
            let (mut t, mut v) = (Vec::new(), Vec::new());
            lab.world.transforms(&[chassis], &mut t);
            lab.world.velocities(&[chassis], &mut v);
            let pace = v[0].linear.length();
            let spin = lab.world.wheel_spins(vehicle)[..2]
                .iter()
                .fold(0.0_f32, |a, s| a.max(s.abs()));
            (t[0].position.z, pace, spin * yard::WHEEL_RADIUS - pace)
        };
        // Its first run, past its sand and stopped beyond the beds; then back to the start.
        for _ in 0..900 {
            lab.tick(&[]);
        }
        let reset = Stamped {
            tick: 900,
            player: 0,
            seq: 0,
            command: LabCommand::Reset,
        };
        lab.tick(&[reset]);
        // Its second run, from that tick: it stands in its sand again until `LAUNCH` (#195), then
        // floored, its driven wheels spin there.
        for _ in 1..yard::LAUNCH {
            lab.tick(&[]);
        }
        let (z, pace, _) = car(&lab);
        assert!(
            (yard::SAND_END..yard::HALT).contains(&z) && pace < 0.05,
            "{pace} m/s at {z}"
        );
        let mut slip = f32::MIN;
        for _ in 0..60 {
            lab.tick(&[]);
            slip = slip.max(car(&lab).2);
        }
        assert!(slip > 5.0, "the treads ran {slip} m/s past the car");
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
    fn the_bridge_stands_and_falls_under_the_convoy_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Bridge, test_pool()).unwrap();
        let convoy = first.convoy.clone().unwrap();
        let all = convoy.deck.joints.len();
        // A second untouched: the deck stands and the cars wait.
        for _ in 0..60 {
            first.tick(&[]);
        }
        assert_eq!(convoy.deck.holding(&first.world), all);
        assert!(convoy.held(&first.world));
        // Let go: the deck carries the first car, then gives way with the second on it too (about
        // 4.7 s on at the time of writing); most of its joints break, the two cars on it fall
        // into the gap, and the two behind stop on the near bank.
        let commands = vec![Stamped {
            tick: 60,
            player: 0,
            seq: 0,
            command: LabCommand::Release,
        }];
        let recording = Recording::record(&mut first, commands, 600, 60);
        let holding = convoy.deck.holding(&first.world);
        assert!(holding < all / 2, "{holding} of {all} joints holding");
        assert_eq!(convoy.down(&first.world), 2);
        assert_eq!(convoy.across(&first.world), 0);
        let (mut second, _) = LabWorld::new(LabScene::Bridge, test_pool()).unwrap();
        for _ in 0..60 {
            second.tick(&[]);
        }
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_rocket_climbs_as_its_thrust_says_tips_on_the_stick_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Rocket, test_pool()).unwrap();
        let body = first.rocket.unwrap();
        let at = |lab: &LabWorld| {
            let (mut t, mut v) = (Vec::new(), Vec::new());
            lab.world.transforms(&[body], &mut t);
            lab.world.velocities(&[body], &mut v);
            (t[0], v[0])
        };
        // Half a second untouched: it stands on its pad.
        for _ in 0..30 {
            first.tick(&[]);
        }
        let (start, _) = at(&first);
        assert!((start.rotation * Vec3::Y).y > 0.9999);
        let fly = |tick: u64, controls: [f32; 4]| Stamped {
            tick,
            player: 0,
            seq: 0,
            command: LabCommand::Fly { controls },
        };
        // Full throttle for 5 s: 50 kN against 3 t climbs at 6.86 m/s² (the drag under 300 N),
        // 86 m, straight up.
        let climb = Recording::record(&mut first, vec![fly(30, [1.0, 0.0, 0.0, 0.0])], 300, 60);
        let (t, v) = at(&first);
        let height = t.position.y - start.position.y;
        assert!((82.0..88.0).contains(&height), "{height:.1} m up");
        assert!((t.rotation * Vec3::Y).y > 0.999, "{:?}", t.rotation);
        assert!(
            v.linear.x.abs() < 0.1 && v.linear.z.abs() < 0.1,
            "{:?}",
            v.linear
        );
        // The stick pushed for a second, then let go: it tips downrange (−z) and flies on that
        // way, the fins keeping it into its wind.
        let commands = vec![
            fly(330, [1.0, 0.5, 0.0, 0.0]),
            fly(390, [1.0, 0.0, 0.0, 0.0]),
        ];
        let turn = Recording::record(&mut first, commands, 300, 60);
        let (t, v) = at(&first);
        let axis = t.rotation * Vec3::Y;
        assert!(
            axis.z < -0.05 && v.linear.z < -2.0,
            "{axis:?} {:?}",
            v.linear
        );
        assert!(axis.x.abs() < 0.01, "{axis:?}");
        let (mut second, _) = LabWorld::new(LabScene::Rocket, test_pool()).unwrap();
        for _ in 0..30 {
            second.tick(&[]);
        }
        climb.replay(&mut second).expect("the same digests");
        turn.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_ship_in_zero_g_keeps_momentum_through_its_crash_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Space, test_pool()).unwrap();
        let ship = first.ship.unwrap();
        // The bodies: the ship, then the 27 crates.
        let crates = first.bodies[1..=space::CRATES].to_vec();
        let momentum = |lab: &LabWorld| space::momentum(&lab.world, ship, &crates);
        // Half a second untouched: nothing moves.
        for _ in 0..30 {
            first.tick(&[]);
        }
        assert_eq!(momentum(&first), Vec3::ZERO);
        let fly = |tick: u64, controls: [f32; 4]| Stamped {
            tick,
            player: 0,
            seq: 0,
            command: LabCommand::Fly { controls },
        };
        // Full throttle for a second: 60 kN for 1 s gives 60 000 kg·m/s along −z (15 m/s), then
        // it coasts; the flight assist's jets turn it but push nothing.
        let commands = vec![fly(30, [1.0, 0.0, 0.0, 0.0]), fly(90, [0.0; 4])];
        let burn = Recording::record(&mut first, commands, 90, 30);
        let before = momentum(&first);
        assert!((before.z + ship::THRUST).abs() < 5.0, "{before}");
        assert!(before.x.abs() < 0.1 && before.y.abs() < 0.1, "{before}");
        // Coasting into the crates (about 1.2 s on) and scattering them: 4 s on, the momentum is
        // what it was, shared out.
        let crash = Recording::record(&mut first, Vec::new(), 240, 60);
        let after = momentum(&first);
        assert!(
            (after - before).length() < 1e-4 * before.length(),
            "{before} {after}"
        );
        let mut v = Vec::new();
        first.world.velocities(&crates, &mut v);
        let moving = v.iter().filter(|v| v.linear.length() > 1.0).count();
        assert!(moving > 10, "{moving} crates moving");
        let (mut second, _) = LabWorld::new(LabScene::Space, test_pool()).unwrap();
        for _ in 0..30 {
            second.tick(&[]);
        }
        burn.replay(&mut second).expect("the same digests");
        crash.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn the_ship_turns_at_the_rate_its_stick_asks_and_holds_still_when_let_go() {
        let (mut lab, _) = LabWorld::new(LabScene::Space, test_pool()).unwrap();
        let ship = lab.ship.unwrap();
        let spin = |lab: &LabWorld| {
            let (mut t, mut v) = (Vec::new(), Vec::new());
            lab.world.transforms(&[ship], &mut t);
            lab.world.velocities(&[ship], &mut v);
            t[0].rotation.inverse() * v[0].angular
        };
        let fly = |tick: u64, controls: [f32; 4]| Stamped {
            tick,
            player: 0,
            seq: 0,
            command: LabCommand::Fly { controls },
        };
        // Half the stick to the right for two seconds: it rolls right (about its nose, −z) at
        // half its most rate, 0.7 rad/s, and turns about nothing else.
        lab.tick(&[fly(0, [0.0, 0.0, 0.5, 0.0])]);
        for _ in 0..120 {
            lab.tick(&[]);
        }
        let rolling = spin(&lab);
        assert!((rolling.z + 0.7).abs() < 0.02, "{rolling}");
        assert!(
            rolling.x.abs() < 0.01 && rolling.y.abs() < 0.01,
            "{rolling}"
        );
        // Let go: within a second and a half the jets have stopped it.
        lab.tick(&[fly(lab.tick, [0.0; 4])]);
        for _ in 0..90 {
            lab.tick(&[]);
        }
        let still = spin(&lab);
        assert!(still.length() < 0.01, "{still}");
    }

    #[test]
    fn the_tug_of_war_goes_to_the_stronger_team_and_replays() {
        let (mut first, _) = LabWorld::new(LabScene::Tug, test_pool()).unwrap();
        let sled = first.tug.unwrap();
        let x = |lab: &LabWorld| {
            let mut t = Vec::new();
            lab.world.transforms(&[sled], &mut t);
            t[0].position.x
        };
        // A second with both teams holding: the sled stays on the middle line.
        for _ in 0..60 {
            first.tick(&[]);
        }
        assert!(x(&first).abs() < 0.01, "{}", x(&first));
        assert_eq!(tug::winner(&first.world, sled), None);
        // The left team pulls with all its strength: 2 000 N against the right's 1 000 and the
        // sled's 785 N of friction pulls it over the left line in about 2.4 s.
        let commands = vec![Stamped {
            tick: 60,
            player: 0,
            seq: 0,
            command: LabCommand::Pull { strength: 1.0 },
        }];
        let recording = Recording::record(&mut first, commands, 180, 60);
        assert_eq!(tug::winner(&first.world, sled), Some(0), "{}", x(&first));
        let (mut second, _) = LabWorld::new(LabScene::Tug, test_pool()).unwrap();
        for _ in 0..60 {
            second.tick(&[]);
        }
        recording.replay(&mut second).expect("the same digests");
    }

    #[test]
    fn a_tug_of_war_over_a_lossy_link_ends_where_the_server_is() {
        let mut net = Net::new(LabScene::Tug, 100.0, &test_pool()).unwrap();
        // This player eases at its tick 30 and pulls harder at 400; the bot pulls hard
        // and eases off every 1.5 s until its tick 600 (six changes), then everyone runs on.
        let mine = [(30, 0.4), (400, 0.8)];
        let mut queued = Vec::new();
        for k in 0..720 {
            net.bot_on = k < 600;
            let now = net.client.sim.now();
            queued.extend(
                mine.iter()
                    .filter(|&&(tick, _)| tick == now)
                    .map(|&(_, strength)| LabCommand::Pull { strength }),
            );
            net.tick(&mut queued);
        }
        // Every command taken in time: this player's two and the bot's six.
        assert_eq!(net.server.stats.applied, 2 + 6);
        assert_eq!(net.server.stats.late, 0);
        // The client predicts the sled until the bot changes its pull, which it learns a link
        // late: corrected then, to the bit otherwise.
        let c = net.client.stats;
        assert!(c.matched > 0 && c.corrected >= 6, "{c:?}");
        let state = net.server.sim.save();
        let (mut server, _) = LabWorld::new(LabScene::Tug, test_pool()).unwrap();
        server.restore(&state);
        while server.now() < net.client.sim.now() {
            server.tick(&[]);
        }
        assert_eq!(server.digest(), net.client.sim.digest());
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

    #[test]
    fn the_tanks_hold_their_water_back_for_space() {
        // Space lets go only what the scene says it holds back: the tank's gate was left out,
        // and Space threw a ball at it (#156, the owner's report).
        for kind in [
            LabScene::Tank,
            LabScene::TankBench,
            LabScene::TankHole,
            LabScene::TankBlocks,
        ] {
            let (mut lab, _) = LabWorld::new(kind, test_pool()).unwrap();
            let tank = lab.tank.clone().unwrap();
            lab.tick(&[]);
            assert!(tank.closed(&lab.world), "{kind:?}");
            let release = Stamped {
                tick: lab.now(),
                player: 0,
                seq: 0,
                command: LabCommand::Release,
            };
            lab.tick(&[release]);
            lab.tick(&[]);
            assert!(!tank.closed(&lab.world), "{kind:?}");
        }
    }
}

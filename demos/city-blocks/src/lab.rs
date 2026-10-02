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
use std::time::Instant;

use anyhow::{Context as _, Result};
use forge_app::Context;
use forge_geom::city::{Block, Lathe, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, Shape, Transform, Velocity, World, WorldDesc};
use forge_render::meshlet::MeshId;
use forge_render::{MeshletScene, MeshletSceneBuilder, MoverTransform};
use forge_sim::{
    Client, Codec, InputPacket, Link, LinkParams, PlayerId, Recording, Server, Simulation,
    Snapshot, Stamped, TICK, TICK_RATE,
};
use glam::{DVec3, Mat4, Quat, Vec3};

use super::{Args, CityMaterials, Cooked, barrel_prop, scene_origin};

/// The lab's scenes (`--lab`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum LabScene {
    /// A pyramid of blocks under a rain of barrels, rocks and balls.
    Drop,
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
/// Ticks between the digests a recording keeps.
const RECORD_EVERY: u64 = 60;
/// `--net`: ticks between the server's snapshots, the share of packets lost, ticks between the
/// bot's throws.
const SNAPSHOT_EVERY: u64 = 6;
const NET_LOSS: f64 = 0.02;
const BOT_EVERY: u64 = 150;

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
            _ => None,
        }
    }
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
/// balls the players throw, and the state the scene started from.
pub(crate) struct LabWorld {
    world: World,
    bodies: Vec<BodyId>,
    thrown: Vec<BodyId>,
    next_throw: u32,
    tick: u64,
    start: Vec<u8>,
}

impl LabWorld {
    /// The scene `kind` from scratch: the same calls in the same order every time, so two built
    /// alike are the same world to the bit. Also the groups the movers draw.
    pub(crate) fn new(kind: LabScene) -> Result<(Self, Vec<Group>)> {
        let mut world = World::new(&WorldDesc {
            // The client's rule (D-005): the cores less two, the caller one of them.
            threads: (std::thread::available_parallelism().map_or(4, |n| n.get()) / 2)
                .saturating_sub(3)
                .max(1) as u32,
            ..WorldDesc::default()
        });
        let floor_shape = Shape::cuboid(Vec3::new(FLOOR_HALF, 0.5, FLOOR_HALF), 0.05, 0.0)?;
        world.add_body(&BodyDesc::fixed(&floor_shape, DVec3::new(0.0, -0.5, 0.0)))?;
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

        let mut bodies = Vec::new();
        let mut groups = Vec::new();
        let mut group = |prop: usize, ids: Vec<BodyId>, bodies: &mut Vec<BodyId>| {
            groups.push(Group {
                prop,
                count: ids.len() as u32,
            });
            bodies.extend(ids);
        };
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
                // from 8 to 45 m up, turned and spinning at random.
                let rain = |k: u64| {
                    let (r, h) = (unit(3 * k), unit(3 * k + 2));
                    // A way without trigonometry: a point of the square folded onto the circle
                    // by its length (a square root only).
                    let (x, z) = (
                        2.0 * unit(3 * k + 1) - 1.0,
                        2.0 * unit(3 * k + 1_000_003) - 1.0,
                    );
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
                // The balls to throw, asleep out of sight until thrown, after the rain's.
                let mut thrown = Vec::new();
                for n in 0..THROWN {
                    thrown.push(world.add_body(&BodyDesc {
                        friction: 0.6,
                        restitution: 0.6,
                        angular_damping: 0.3,
                        ccd: true,
                        asleep: true,
                        ..BodyDesc::dynamic(
                            &ball_shape,
                            DVec3::new(f64::from(n) * 2.0, PARKED_Y, 0.0),
                        )
                    })?);
                }
                balls.extend(&thrown);
                group(BALL, balls, &mut bodies);
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
                    },
                    groups,
                ))
            }
        }
    }

    /// Its bodies' transforms, in the movers' order, into `out`.
    pub(crate) fn transforms(&self, out: &mut Vec<Transform>) {
        self.world.transforms(&self.bodies, out);
    }

    /// Bodies, and those awake.
    pub(crate) fn bodies(&self) -> usize {
        self.bodies.len()
    }

    pub(crate) fn awake(&self) -> u32 {
        self.world.active_bodies()
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
                }
            }
        }
        if let Err(e) = self.world.step(TICK, 1) {
            tracing::warn!("physics tick {}: {e}", self.tick);
        }
        self.tick += 1;
    }

    fn now(&self) -> u64 {
        self.tick
    }

    fn save(&mut self) -> Vec<u8> {
        let mut out = self.tick.to_le_bytes().to_vec();
        out.extend_from_slice(&self.next_throw.to_le_bytes());
        out.extend(self.world.save_state());
        out
    }

    fn restore(&mut self, state: &[u8]) {
        let (tick, rest) = state.split_at(8);
        let (next, world) = rest.split_at(4);
        self.tick = u64::from_le_bytes(tick.try_into().expect("8 bytes"));
        self.next_throw = u32::from_le_bytes(next.try_into().expect("4 bytes"));
        if let Err(e) = self.world.restore_state(world) {
            tracing::warn!("a lab state: {e}");
        }
    }

    fn digest(&mut self) -> u64 {
        self.world.digest(&self.bodies) ^ self.tick.rotate_left(17) ^ u64::from(self.next_throw)
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
    fn new(kind: LabScene, delay_ms: f64) -> Result<Self> {
        let lead = (delay_ms / 1e3 / f64::from(TICK)).ceil() as u64 + 2;
        // A client runs ahead through its own steps, so it knows the digests of those ticks.
        let ahead = |player: PlayerId| -> Result<Client<LabWorld>> {
            let mut client = Client::new(LabWorld::new(kind)?.0, player);
            for _ in 0..lead {
                client.step();
            }
            Ok(client)
        };
        let link = |seed| LinkParams::new(delay_ms, NET_LOSS, seed);
        Ok(Self {
            server: Server::new(LabWorld::new(kind)?.0),
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
        world: LabWorld,
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
    logged: [u64; 3],
    /// `--record`: where the session goes at exit.
    record: Option<PathBuf>,
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
    CityMaterials::new(&ctx.device)?.apply(&mut builder, &props, &ids);
    builder.set_ray_traced(!args.no_shadows);
    builder.set_origin(scene_origin(args));
    builder.add_instance(
        ids[FLOOR],
        Mat4::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    );
    let (world, groups) = LabWorld::new(kind)?;
    let per_mesh: Vec<(MeshId, u32)> = groups.iter().map(|g| (ids[g.prop], g.count)).collect();
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
        Mode::Net(Box::new(Net::new(kind, delay_ms)?))
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
            world,
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
            logged: [60, 300, 600],
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
                tracing::info!(
                    tick = now,
                    digest = format!("{digest:#018x}"),
                    awake,
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
        for c in [throws(1, 1)[0].command, LabCommand::Reset] {
            let mut bytes = Vec::new();
            c.encode(&mut bytes);
            let mut read = bytes.as_slice();
            assert_eq!(LabCommand::decode(&mut read), Some(c));
            assert!(read.is_empty());
        }
    }

    #[test]
    fn the_lab_replays_a_recording_to_the_same_digests() {
        let (mut first, _) = LabWorld::new(LabScene::Drop).unwrap();
        let mut commands = throws(6, 40);
        commands.push(Stamped {
            tick: 200,
            player: 0,
            seq: 6,
            command: LabCommand::Reset,
        });
        let recording = Recording::record(&mut first, commands, 360, 60);
        let (mut second, _) = LabWorld::new(LabScene::Drop).unwrap();
        recording.replay(&mut second).expect("the same digests");
        // And a world that saw one throw fewer leaves it.
        let mut fewer = recording.clone();
        fewer.commands.remove(2);
        let (mut third, _) = LabWorld::new(LabScene::Drop).unwrap();
        assert!(fewer.replay(&mut third).is_err());
    }

    #[test]
    fn a_session_over_a_lossy_link_ends_where_the_server_is() {
        let mut net = Net::new(LabScene::Drop, 100.0).unwrap();
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
        let (mut server, _) = LabWorld::new(LabScene::Drop).unwrap();
        server.restore(&state);
        while server.now() < net.client.sim.now() {
            server.tick(&[]);
        }
        assert_eq!(server.digest(), net.client.sim.digest());
    }
}

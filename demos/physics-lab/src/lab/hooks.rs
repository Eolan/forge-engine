//! The lab in the shared app (#216): its flags, and its scenes as the app's
//! [`Scenario`] and [`Running`]: how each is lit, where its camera starts, its liquid and its
//! water, and frame by frame the keys and the commands they send its world, the camera that
//! follows its player or its vehicle, and the movers that draw its bodies.

use std::path::PathBuf;

use anyhow::Result;
use forge_app::{FlyCamera, Input, Setup};
use forge_geom::city::PropSpec;
use forge_render::{LiquidLook, LiquidSolver, LiquidStep, MeshletScene, SplashSource};
use glam::Vec3;
use winit::keyboard::KeyCode;

use super::{ArmPose, Lab, LabScene, models, room, sounds, tank, yard};
use city_blocks::scenario::{Backdrop, LiquidSetup, Look, PoolView, Running, Scenario, Water};
use city_blocks::{Args, Cooked};

/// `--lab space`'s sun: from the ship's right and a little behind it and above, so the chase
/// camera sees its lit side and the planet's day side with its terminator far to the left.
const SPACE_SUN: Vec3 = Vec3::new(0.75, 0.35, 0.5);
/// The finest probe cascade's spacing in the models lab's rooms, metres (the city's is 4). In
/// the courtyard room, a few metres wide, probes 4 m apart stood in the columns and deep in the
/// arcades: a curtain in the shade beside the sunlit courtyard got about 12 lux of bounced
/// light, and the meter lifted the whole view to show it. At 1 m it gets several times that,
/// the view meters 1.8 stops darker, and fewer pixels shimmer (16 % against 28 % over TAA's
/// cycle), for 0.17 ms more at 1600 × 900.
const ROOM_PROBE_SPACING: f32 = 1.0;
/// The tank's bench (#156): towards its sun (high, from the left and behind), and its background's
/// albedo (a dull violet, after Sebastian Lague's fluid renders).
const BENCH_SUN: Vec3 = Vec3::new(-0.45, 0.8, -0.4);
const BENCH_BACKGROUND: Vec3 = Vec3::new(0.09, 0.07, 0.1);

/// The lab's flags (`physics-lab`, #136): the scene, and what drives it and how it is drawn.
#[derive(clap::Args, Debug, Clone)]
pub(crate) struct LabArgs {
    /// Show one of `physics-lab`'s scenes instead of the city (issue #136): rigid bodies on a
    /// flat floor through `forge-physics`.
    #[arg(long = "lab", value_enum)]
    pub(crate) scene: Option<LabScene>,
    /// `--lab models`: show this model alone (its name as `tools/fetch-assets.sh` lists it, e.g.
    /// `WaterBottle`), framed for a capture beside Khronos's screenshot (#170, D-048).
    #[arg(long)]
    pub(crate) model: Option<String>,
    /// With `--lab`, write the session's commands and digests to this file at exit (#137). With
    /// `--net`, the server's: every player's commands as it applied them, and its whole state
    /// every 10 s, so `--seek` can start the replay anywhere (#268).
    #[arg(long)]
    pub(crate) record: Option<PathBuf>,
    /// With `--lab`, play a recorded session again instead of the keys, checking its digests.
    #[arg(long)]
    pub(crate) replay: Option<PathBuf>,
    /// With `--replay`, start at this tick: the recording's last keyframe before it restored and
    /// the ticks between run before the first frame (a recording made with `--net`).
    #[arg(long, requires = "replay")]
    pub(crate) seek: Option<u64>,
    /// With `--lab`, run the scene through a server and this player's client over a link of
    /// this many milliseconds one way, 2 % of the packets lost, a bot throwing too (#137).
    #[arg(long)]
    pub(crate) net: Option<f64>,
    /// With `--net`, the players reach the server over QUIC on loopback through the network
    /// thread (`forge_net::NetThread`), the link conditioner both ways, on the wall clock (#268).
    #[arg(long, requires = "net")]
    pub(crate) net_quic: bool,
    /// With `--lab`, throw a ball from the camera every this many frames, as Space does.
    #[arg(long)]
    pub(crate) throw_every: Option<u64>,
    /// With `--lab`, shoot from the camera every this many frames: the body it meets is pushed,
    /// a creature's part struck (Phase 7's hit reactions).
    #[arg(long)]
    pub(crate) shoot_every: Option<u64>,
    /// With `--lab sea`, the boat's throttle and rudder from the first frame, `T,R` (−1 to 1),
    /// in place of the arrow keys (#138).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    pub(crate) steer: Option<Vec<f32>>,
    /// With `--lab fly`, the aeroplane's controls from the first frame, `T,E,A,R` (throttle
    /// 0 to 1, elevator, ailerons and rudder −1 to 1), in place of the keys (#141).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    pub(crate) pilot: Option<Vec<f32>>,
    /// With `--lab creatures`, let the creatures' motors go at this frame, as ↓ does (#143).
    #[arg(long)]
    pub(crate) limp_at: Option<u64>,
    /// At this frame, as Space does: the wrecking ball let go (`--lab break`, #142), the flood's
    /// gate lifted (#144), the first domino tipped (#146), the convoy sent across (#147). The
    /// glass tank's gate lifted (#156).
    #[arg(long)]
    pub(crate) release: Option<u64>,
    /// `--lab tank` (#156): the liquid's pressure sweeps a substep (red and black each), without
    /// `--liquid-cycles`.
    #[arg(long, default_value_t = 32)]
    pub(crate) liquid_sweeps: u32,
    /// `--lab tank`: the liquid's cell, metres (590 000 particles at 1.25 cm, the default; 1.15
    /// million at 1 cm).
    #[arg(long, default_value_t = tank::CELL)]
    pub(crate) liquid_cell: f32,
    /// `--lab tank`: the liquid's gravity, m/s² (x,y,z; `0,0,0` for none). Without surface
    /// tension, none leaves the water as it stands.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true, default_values_t = [0.0, -9.81, 0.0])]
    pub(crate) liquid_gravity: Vec<f32>,
    /// `--lab tank`: what the tank's drawing shows: the water as it looks (key 1), its speed view
    /// (the surface coloured by the flow's speed; key 2) or its landing view (where its bent rays
    /// land; key 3).
    #[arg(long, value_enum, default_value_t = LiquidView::Look)]
    pub(crate) liquid_view: LiquidView,
    /// `--lab tank`: how long the air the water takes in lasts, seconds (fresh water's 0.3 by
    /// default: white only where a jet plunges; longer for sea water's foam; 0 for none).
    #[arg(long, default_value_t = forge_render::FRESH_FOAM_LIFE)]
    pub(crate) liquid_foam: f32,
    /// `--lab tank`: the share of the particles' crowding undone a substep.
    #[arg(long, default_value_t = 0.25)]
    pub(crate) liquid_drift: f32,
    /// `--lab tank`: the pressure sweeps' over-relaxation.
    #[arg(long, default_value_t = 1.7)]
    pub(crate) liquid_omega: f32,
    /// `--lab tank`: multigrid V-cycles of the liquid's pressure a substep, in place of the
    /// sweeps (0: the sweeps).
    #[arg(long, default_value_t = 0)]
    pub(crate) liquid_cycles: u32,
    /// `--lab tank`: with `--liquid-cycles`, the red-black sweeps before and after each level's
    /// correction.
    #[arg(long, default_value_t = 2)]
    pub(crate) liquid_smooth: u32,
    /// `--lab tank`: their over-relaxation.
    #[arg(long, default_value_t = 1.0)]
    pub(crate) liquid_smooth_omega: f32,
    /// `--lab tank`: the lab's ticks between the liquid's lines in the log.
    #[arg(long, default_value_t = 60)]
    pub(crate) liquid_log: u64,
    /// Skin the lab's creatures and gulls by dual quaternions, not linear blending: a twisting
    /// joint keeps its thickness, a bending one swells a little (#169, D-052; on request only).
    #[arg(long)]
    pub(crate) dual_quaternion: bool,
    /// `--lab creatures`: draw the mannequins' left forearm twisted or bent 90° on top of their
    /// pose, to compare the skinning's blends (#169).
    #[arg(long, value_enum)]
    pub(crate) arm_pose: Option<ArmPose>,
    /// Give the lab's mannequins corrective morph targets at the elbows, weighted by how far
    /// each bends: the linear blend's bent elbow keeps its thickness (#169; on request only).
    #[arg(long)]
    pub(crate) elbow_correctives: bool,
    /// `--lab walk`: run this mission on the playground (a RON record, D-072, #287): its
    /// objectives in the title and the log, its items drawn as barrels, its state in the lab's
    /// saves, digests and recordings. The file is read again when it changes, the mission
    /// keeping its stage.
    #[arg(long)]
    pub(crate) mission: Option<PathBuf>,
    /// With `--mission`, the player walks to the mission's first target in turn, as a scripted
    /// run's player (its walks are commands, recorded and replayed).
    #[arg(long)]
    pub(crate) mission_autopilot: bool,
    /// Skin the lab's slimes on the eight soft-body points nearest each vertex, smoothly
    /// weighted, not on the three of the soft body's triangle under it: their surface bends
    /// smoothly across the soft body's edges (#169's eight joints a vertex; on request only).
    #[arg(long)]
    pub(crate) slime_eight: bool,
}

/// `--mission-autopilot`'s walking speed, m/s.
const AUTOPILOT_SPEED: f64 = 1.5;

/// The walk from the player's feet towards the mission's target (`--mission-autopilot`, #287):
/// along the ground at a walk, none within 20 cm of it or without a target.
fn autopilot(target: Option<(glam::DVec3, glam::DVec3)>) -> [f32; 2] {
    let Some((at, to)) = target else {
        return [0.0; 2];
    };
    let way = glam::DVec2::new(to.x - at.x, to.z - at.z);
    if way.length() < 0.2 {
        return [0.0; 2];
    }
    let v = way.normalize() * AUTOPILOT_SPEED;
    [v.x as f32, v.y as f32]
}

/// `--liquid-view`: what the tank's drawing shows (#156).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum LiquidView {
    /// The water as it looks.
    Look,
    /// The speed view: the surface matte, coloured by the flow's speed.
    Speed,
    /// The landing view: each pixel of water coloured by where its bent ray lands.
    Landing,
}

impl LiquidView {
    fn mode(self) -> forge_render::LiquidMode {
        match self {
            Self::Look => forge_render::LiquidMode::Look,
            Self::Speed => forge_render::LiquidMode::Speed,
            Self::Landing => forge_render::LiquidMode::Landing,
        }
    }
}

/// One of the lab's scenes, before it is built.
pub(crate) struct LabScenario {
    kind: LabScene,
    flags: LabArgs,
}

impl LabScenario {
    /// The scene `kind` with the lab's `flags`. The models scene's model is set here, once a
    /// process (`--model`).
    pub(crate) fn new(kind: LabScene, flags: LabArgs) -> Self {
        models::FOCUS
            .set(flags.model.clone())
            .expect("the models scene's model, set once");
        Self { kind, flags }
    }
}

impl Scenario for LabScenario {
    fn props(&self) -> Vec<PropSpec> {
        super::props(self.kind)
    }

    fn look(&self) -> Look {
        let kind = self.kind;
        Look {
            // In space (the owner's ask of 2026-10-03): the sun unfiltered by any air, from the
            // right and behind the ship. The tank's bench (#156): a white sun from the left and
            // behind. The sharpness room (#159): the sun alone, white, from the front left. The
            // yard (#185) and the materials' patches (#205): a low sun across the beds, the
            // prints in raking light.
            sun: match kind {
                LabScene::Space => Some(SPACE_SUN),
                LabScene::TankBench => Some(BENCH_SUN),
                LabScene::Room => Some(room::SUN),
                LabScene::Yard | LabScene::Materials => Some(yard::SUN),
                _ => None,
            },
            white_sun: matches!(kind, LabScene::Space | LabScene::TankBench | LabScene::Room),
            backdrop: match kind {
                LabScene::Space => Backdrop::Space,
                LabScene::TankBench => Backdrop::Card(BENCH_BACKGROUND),
                LabScene::Room => Backdrop::Indoors,
                _ => Backdrop::Sky,
            },
            // The yard's spray (#192).
            splashes: kind == LabScene::Yard,
            // The smoke yard's chimneys, fire and footsteps (D-064).
            particles: kind == LabScene::Smoke,
            // The models lab's rooms, lit through a few openings and seen at an indoor
            // exposure: 256 rays, probes a metre apart (#171).
            // An authored scene's the finest 2 m apart in six cascades (D-083): at 4 m the
            // street's light reached under the Bistro's awnings, their undersides 3 to 6 times
            // Falcor's (1.5 to 2 at 1 m, for 0.65 ms). Under D-088's bounce, which lights what
            // the camera sees, 2 m matched the path as well (an awning 1.08 either way) and
            // nearer in Exterior_2 (0.185 against 0.219), for 0.56 ms less. Chosen at load, so
            // the probes alone (`r.gi.trace 0`, no NRD) are 2 m apart too.
            probes: if kind == LabScene::Models && models::room_shown() {
                Some((
                    256,
                    ROOM_PROBE_SPACING,
                    forge_render::probes::ProbeParams::default().cascades,
                ))
            } else if kind.authored().is_some() {
                Some((forge_render::probes::ProbeParams::default().rays, 2.0, 6))
            } else {
                None
            },
            // The lab's bodies are what it shows: the probes see them (#299).
            probes_see_movers: true,
            probe_far_room_sky: None,
            probe_far_room_daylight: None,
            probe_far_room_lamps: None,
            probe_room_grids: None,
            // The car is followed from the start (C lets it go); the aeroplane always is.
            chase: matches!(kind, LabScene::Drive | LabScene::Flyer),
            // Each lab scene at its own fixed exposure (its captures compared at one), but the
            // Bistro: metered as a street walked under its café's awnings, half a stop brighter
            // than the shared meter by day, between its authors' two pictures (D-089).
            // `--fixed-exposure` keeps a comparison at one.
            metered: kind == LabScene::Bistro,
            exposure_compensation: (kind == LabScene::Bistro)
                .then_some(super::bistro::EXPOSURE_COMPENSATION),
            local_exposure: false,
            // An authored scene under its authors' sky and sun (D-084), the chart under its own, for
            // comparisons with Falcor's renders.
            environment: kind.authored().and_then(super::bistro::environment),
            // Falcor's renders are tone-mapped by its ToneMapper: Narkowicz's ACES (D-084).
            tonemap: kind
                .authored()
                .map(|_| forge_render::Tonemap::AcesNarkowicz),
        }
    }

    fn liquid(&self) -> Option<LiquidSetup> {
        // The glass tank's liquid (#156): pure water, the solver as the flags set it; on its
        // bench, tinted.
        let kind = self.kind;
        let f = &self.flags;
        let bench = kind == LabScene::TankBench;
        matches!(
            kind,
            LabScene::Tank | LabScene::TankBench | LabScene::TankHole | LabScene::TankBlocks
        )
        .then(|| LiquidSetup {
            tank: tank::liquid(
                f.liquid_cell,
                kind == LabScene::TankHole,
                kind == LabScene::TankBlocks,
            ),
            solver: LiquidSolver {
                sweeps: f.liquid_sweeps,
                omega: f.liquid_omega,
                cycles: f.liquid_cycles,
                smooth: f.liquid_smooth,
                smooth_omega: f.liquid_smooth_omega,
                drift: f.liquid_drift,
                gravity: Vec3::from_slice(&[f.liquid_gravity.as_slice(), &[0.0; 3]].concat()),
                ..LiquidSolver::default()
            },
            look: LiquidLook {
                foam_life: f.liquid_foam,
                ..if bench {
                    LiquidLook::tinted()
                } else {
                    LiquidLook::pure_water()
                }
            },
            mode: f.liquid_view.mode(),
            corner: tank::corner(bench),
            log_every: f.liquid_log,
        })
    }

    fn camera(&self) -> FlyCamera {
        if self.kind == LabScene::Fly {
            // Behind the aeroplane on the runway's threshold; it follows the aeroplane.
            FlyCamera {
                position: Vec3::new(0.0, 4.0, 210.0),
                yaw: 0.0,
                pitch: -0.15,
                speed: 20.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Tug {
            // In front of the sled, the three lines and most of the ropes in view: the left team's to
            // the left.
            FlyCamera {
                position: Vec3::new(0.0, 2.6, 8.0),
                yaw: 0.0,
                pitch: -0.2,
                speed: 6.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Rocket {
            // Off the rocket's right on the pad, looking at it; it follows the rocket once flown.
            FlyCamera {
                position: Vec3::new(30.0, 8.0, 5.0),
                yaw: 1.406,
                pitch: -0.01,
                speed: 20.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Bridge {
            // Beside the gap, a little over the deck, looking across the bridge from its side: the
            // cars come from the left.
            FlyCamera {
                position: Vec3::new(20.0, 7.0, 2.0),
                yaw: 1.5,
                pitch: -0.15,
                speed: 10.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Dominoes {
            // Over the spiral's outer edge, looking down across it.
            FlyCamera {
                position: Vec3::new(0.0, 6.5, 11.0),
                yaw: 0.0,
                pitch: -0.55,
                speed: 6.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Smoke {
            // At a walker's height behind the footsteps, the fire and the chimneys ahead.
            FlyCamera {
                position: Vec3::new(0.0, 2.2, 7.5),
                yaw: 0.0,
                pitch: 0.05,
                speed: 4.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Models {
            // Before the models' row, or framing the one `--model` names (inside a room model).
            let (position, yaw, pitch) = models::camera();
            FlyCamera {
                position,
                yaw,
                pitch,
                speed: 2.0,
                ..FlyCamera::default()
            }
        } else if let Some(which) = self.kind.authored() {
            // Where the FBX's camera path starts, in the street before the café; above the chart.
            let (position, yaw, pitch) = super::bistro::camera(which);
            FlyCamera {
                position,
                yaw,
                pitch,
                speed: 4.0,
                ..FlyCamera::default()
            }
        } else if matches!(self.kind, LabScene::Room | LabScene::Lamps) {
            // In the sharpness room's middle at eye height, looking level at the back wall's targets
            // 8 m away; the board's, 2.5 m away, below them.
            FlyCamera {
                position: Vec3::new(0.0, 1.5, 3.0),
                yaw: 0.0,
                pitch: 0.0,
                speed: 2.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::TankHole {
            // In front of the tank, right of the gate, a little over the rim: the hole low in the gate
            // and the dry side its jet runs into.
            FlyCamera {
                position: Vec3::new(0.55, 1.3, 1.2),
                yaw: 0.5,
                pitch: -0.25,
                speed: 0.8,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::TankBench {
            // Over the bench's front right, looking down across the tank at its floor of squares.
            FlyCamera {
                position: Vec3::new(1.0, 1.05, 1.5),
                yaw: 0.58,
                pitch: -0.42,
                speed: 0.8,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::TankBlocks {
            // In front of the tank's right half, over its rim, looking down at the cube and the posts
            // the wave meets past the gate.
            FlyCamera {
                position: Vec3::new(0.6, 1.3, 0.8),
                yaw: 0.35,
                pitch: -0.55,
                speed: 0.8,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Tank {
            // In front of the tank and to its right, a little over its rim, looking down into it: the
            // reservoir behind the gate on the left.
            FlyCamera {
                position: Vec3::new(0.45, 1.4, 1.6),
                yaw: 0.28,
                pitch: -0.22,
                speed: 0.8,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Flood {
            // Over the basin's lower corner, looking up it at the gate and the reservoir.
            FlyCamera {
                position: Vec3::new(12.0, 9.0, 16.0),
                yaw: 0.9,
                pitch: -0.34,
                speed: 10.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Creatures {
            // Before the creatures, at a man's height, the mannequins behind the dogs.
            FlyCamera {
                position: Vec3::new(0.0, 1.4, 4.4),
                yaw: 0.0,
                pitch: -0.1,
                speed: 6.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Course {
            // Beside the course and over it, looking down across both lanes: the ramps nearer.
            FlyCamera {
                position: Vec3::new(3.3, 2.1, 0.6),
                yaw: std::f32::consts::FRAC_PI_2,
                pitch: -0.52,
                speed: 6.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Yard {
            // Over the end of the car's snow, looking down across its ruts to the dogs' beds (#186).
            FlyCamera {
                position: Vec3::new(5.6, 2.2, 2.6),
                yaw: 60f32.to_radians(),
                pitch: -40f32.to_radians(),
                speed: 6.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Flyer {
            // Outside the gulls' circuit and under it, looking across it: the near ones pass close.
            FlyCamera {
                position: Vec3::new(0.0, 5.0, 45.0),
                yaw: 0.0,
                pitch: 0.05,
                speed: 10.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Break {
            // In front of the wall and to its left, clear of the gantry's post: the wall's face, the
            // column behind it, the ball held back at the right edge.
            FlyCamera {
                position: Vec3::new(-6.5, 2.4, 7.5),
                yaw: -0.71,
                pitch: -0.1,
                speed: 6.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Drive {
            // Behind the car and to its right, the track ahead along −z.
            FlyCamera {
                position: Vec3::new(4.0, 3.0, 8.0),
                yaw: 0.35,
                pitch: -0.15,
                speed: 10.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Materials {
            // Behind the walker's start and to its left, high enough to see the patches along its
            // way, their ramps beyond and the ice at the end (#203).
            FlyCamera {
                position: Vec3::new(-9.0, 7.0, -24.0),
                yaw: -2.79,
                pitch: -0.21,
                speed: 8.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Walk {
            // Behind the player, looking along −z at the ramps, a little down; it follows the
            // player from the first frame.
            FlyCamera {
                position: Vec3::new(0.0, 3.0, 6.0),
                yaw: 0.0,
                pitch: -0.25,
                speed: 8.0,
                ..FlyCamera::default()
            }
        } else if self.kind == LabScene::Sea {
            // Over the jetty, looking out past its end at the water where things fell, the boat on
            // the left.
            FlyCamera {
                position: Vec3::new(-2.0, 6.5, -4.0),
                yaw: -0.42,
                pitch: -0.2,
                speed: 8.0,
                ..FlyCamera::default()
            }
        } else {
            // South-east of the pyramid, a little above its top, looking at it.
            FlyCamera {
                position: Vec3::new(13.0, 7.5, 17.0),
                yaw: 0.65,
                pitch: -0.22,
                speed: 8.0,
                ..FlyCamera::default()
            }
        }
    }

    fn build(
        self: Box<Self>,
        ctx: &Setup,
        args: &Args,
        cooked: Cooked,
    ) -> Result<(MeshletScene, Box<dyn Running>)> {
        let (mut scene, mut lab) = super::build(ctx, args, &self.flags, cooked, self.kind)?;
        lab.arm_pose = self.flags.arm_pose;
        let chase = self.look().chase;
        // `FORGE_DYNAMIC_SCENE=noedit`: the scene built to be edited, no edit made.
        let edits = std::env::var("FORGE_DYNAMIC_SCENE").is_ok_and(|v| v != "noedit");
        let mut editor = match lab.edited {
            Some(_) if edits => scene.editor(&ctx.device, &ctx.shaders)?,
            _ => None,
        };
        // `=cells` (#338 step 4): the editor goes to a cell streamer, whose cells are props in
        // the sky over the pyramid, streamed round a viewer that crosses the view.
        let cells = match (std::env::var("FORGE_DYNAMIC_SCENE").as_deref(), lab.edited) {
            (Ok("cells"), Some(mesh)) => {
                let editor = editor.take().expect("the scene's editor");
                let streamer = forge_render::CellStreamer::new(
                    editor,
                    GridCells {
                        mesh,
                        props: std::env::var("FORGE_CELL_PROPS")
                            .ok()
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(GRID_PROPS),
                        rock: forge_geom::procedural::asteroid(
                            forge_core::Seed::new(7),
                            4,
                            0.5,
                            0.2,
                        ),
                    },
                    GRID,
                    forge_render::CellStreamConfig {
                        // The cells' content within 40 m, their proxies to 96 m (D-077);
                        // `FORGE_CELL_PROXIES=0`: none.
                        policy: forge_world::StreamPolicy {
                            finest: GRID_LEVEL,
                            coarsest: GRID_LEVEL,
                            rings: 2.5,
                            hysteresis: 0.25,
                            proxy_rings: if std::env::var("FORGE_CELL_PROXIES")
                                .is_ok_and(|v| v == "0")
                            {
                                0.0
                            } else {
                                6.0
                            },
                        },
                        cells_per_edit: 8,
                    },
                )?;
                Some((streamer, std::sync::Arc::clone(&ctx.device)))
            }
            _ => None,
        };
        Ok((
            scene,
            Box::new(LabRunning {
                bistro_lamps: super::bistro::lamps_on(super::bistro::night_start(args.time_of_day)),
                kind: self.kind,
                flags: self.flags,
                lab,
                chase,
                steering: (0.0, 0.0),
                walking: [0.0; 2],
                handbrake: false,
                beds_seen: u64::MAX,
                beds_again: false,
                editor,
                edit_frames: 0,
                edit_slot: None,
                edit_at: if std::env::var("FORGE_DYNAMIC_SCENE")
                    .is_ok_and(|v| matches!(v.as_str(), "visible" | "promote" | "swing"))
                {
                    glam::DVec3::new(0.0, 9.0, 0.0)
                } else {
                    glam::DVec3::new(0.0, -1000.0, 0.0)
                },
                promote: match std::env::var("FORGE_DYNAMIC_SCENE").as_deref() {
                    Ok("promote") => Some(false),
                    Ok("swing") => Some(true),
                    _ => None,
                },
                promoted: None,
                cells,
                flying: [0.0; 4],
                pulling: 0.5,
                impacts: None,
                heard: Vec::new(),
                feet: None,
            }),
        ))
    }
}

/// `=cells`' world (#338 step 4): a flat grid of 64 m roots, its cells of [`GRID_LEVEL`] (16 m).
const GRID: forge_world::FlatGrid = forge_world::FlatGrid { root_size: 64.0 };
const GRID_LEVEL: u8 = 2;
/// The props a cell holds by default (`FORGE_CELL_PROPS` gives another count: the editor's cost at
/// a town's scale), at this height over the ground.
const GRID_PROPS: u32 = 4;
const GRID_HEIGHT: f64 = 9.0;

/// `=cells`' source: each cell's props, placed by its id, as a game's cells would come from its
/// data.
struct GridCells {
    mesh: forge_render::MeshId,
    /// Its props a cell.
    props: u32,
    /// What stands for a prop in a cell's proxy (D-077): a rock, so a capture tells them apart.
    rock: forge_geom::TriMesh,
}

impl GridCells {
    /// Prop `k` of `cell`: where it stands and how it turns, from a hash of the cell's id.
    fn place(cell: forge_world::CellId, k: u32) -> (glam::DVec3, f32) {
        use forge_world::Partition;
        let size = GRID.cell_size(cell.level());
        let (x, y) = cell.xy();
        let mut h = cell.0 ^ (u64::from(k) + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h ^= h >> 31;
        h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        h ^= h >> 29;
        let unit = |shift: u32| ((h >> shift) & 0xffff) as f64 / 65_536.0;
        let at = glam::DVec3::new(
            (x as f64 + 0.15 + 0.7 * unit(0)) * size,
            GRID_HEIGHT + 4.0 * unit(16),
            (y as f64 + 0.15 + 0.7 * unit(32)) * size,
        );
        (at, (unit(48) * std::f64::consts::TAU) as f32)
    }
}

impl forge_render::CellSource for GridCells {
    fn load(
        &mut self,
        cell: forge_world::CellId,
        editor: &mut forge_render::meshlet::SceneEditor,
    ) -> forge_gpu::Result<forge_render::LoadedCell> {
        let mut instances = Vec::new();
        for k in 0..self.props {
            let (at, turn) = Self::place(cell, k);
            instances.push(editor.add_instance_at(
                self.mesh,
                forge_render::CellPos::from_f64(at),
                glam::Quat::from_rotation_y(turn),
                1.0,
                forge_core::material::MaterialId(0),
            )?);
        }
        Ok(forge_render::LoadedCell {
            instances,
            meshes: Vec::new(),
            sets: forge_render::CellSets::default(),
        })
    }

    /// The cell's props as one mesh, a rock where each stands (D-077's proxy), cooked on the
    /// worker: one mesh and one instance a far cell.
    fn load_proxy(
        &mut self,
        cell: forge_world::CellId,
        editor: &mut forge_render::meshlet::SceneEditor,
    ) -> forge_gpu::Result<forge_render::LoadedCell> {
        use forge_world::Partition;
        let size = GRID.cell_size(cell.level());
        let (x, y) = cell.xy();
        let origin = glam::DVec3::new(x as f64 * size, GRID_HEIGHT, y as f64 * size);
        let parts: Vec<forge_geom::proxy::Part> = (0..self.props)
            .map(|k| {
                let (at, turn) = Self::place(cell, k);
                forge_geom::proxy::Part {
                    mesh: &self.rock,
                    at: (at - origin).as_vec3(),
                    rotation: glam::Quat::from_rotation_y(turn),
                    scale: 1.0,
                    material: 0,
                }
            })
            .collect();
        let (merged, row) = forge_geom::proxy::merge(&parts)
            .map_err(|e| forge_gpu::GpuError::Unsupported(format!("a cell's proxy: {e}")))?;
        let cooked = forge_geom::MeshletMesh::build(&merged);
        let mesh = editor.add_mesh(
            &cooked,
            forge_render::meshlet::MeshRays {
                budget: merged.triangle_count() as u32,
                terrain: false,
            },
        )?;
        let instance = editor.add_instance_at(
            mesh,
            forge_render::CellPos::from_f64(origin),
            glam::Quat::IDENTITY,
            1.0,
            forge_core::material::MaterialId(row),
        )?;
        Ok(forge_render::LoadedCell {
            instances: vec![instance],
            meshes: vec![mesh],
            sets: forge_render::CellSets::default(),
        })
    }
}

/// A lab's scene, built: its world and the commands last sent it.
struct LabRunning {
    /// Whether an authored scene's glowing pieces light it as lamps (D-087, `bistro::lamps_on`).
    bistro_lamps: bool,
    kind: LabScene,
    flags: LabArgs,
    lab: Lab,
    /// The camera follows the boat, the car or the aeroplane (C, #138).
    chase: bool,
    /// The boat's throttle and rudder last sent (#138).
    steering: (f32, f32),
    /// The player's walk last sent (#139).
    walking: [f32; 2],
    /// The car's handbrake last sent (#140).
    handbrake: bool,
    /// The yard's beds' change count whose heights went up last, and whether they go up once
    /// more, so the frame before's heights match this frame's again (#186).
    beds_seen: u64,
    beds_again: bool,
    /// The aeroplane's controls last sent (#141).
    flying: [f32; 4],
    /// The tug-of-war's pull last sent (#149).
    pulling: f32,
    /// With `FORGE_DYNAMIC_SCENE` (#338's check): the scene's editor, the frames run, and the
    /// slot of the instance its edit added.
    editor: Option<forge_render::meshlet::SceneEditor>,
    edit_frames: u64,
    edit_slot: Option<u32>,
    /// Where the check's instance stands: 1 km under the ground, or in the sky over the
    /// pyramid with `FORGE_DYNAMIC_SCENE=visible`; its move takes it 2.5 m along x.
    edit_at: glam::DVec3,
    /// With `=promote` or `=swing` (#338 piece 5), whether it swings, and its mover while it is
    /// promoted.
    promote: Option<bool>,
    promoted: Option<u32>,
    /// With `=cells` (#338 step 4): the cell streamer, with the device its cells' sets need.
    cells: Option<(
        forge_render::CellStreamer,
        std::sync::Arc<forge_gpu::Device>,
    )>,
    /// Its impacts' sounds, once the app plays sound (Phase 7), and the contacts heard this
    /// frame.
    impacts: Option<sounds::Impacts>,
    heard: Vec<super::Heard>,
    /// Its player's feet as drawn this frame, between its ticks (#318).
    feet: Option<Vec3>,
}

/// The frames at which #338's check adds its instance, moves it, then removes it; with
/// `=promote` or `=swing` it is promoted and demoted where the move takes it, in place of the
/// move.
const EDIT_ADD: u64 = 30;
const EDIT_PROMOTE: u64 = 35;
const EDIT_MOVE: u64 = 45;
const EDIT_DEMOTE: u64 = 50;
const EDIT_REMOVE: u64 = 60;

impl Running for LabRunning {
    fn water(&mut self, ctx: &Setup, args: &Args) -> Result<Option<Water>> {
        Ok(match self.kind {
            // The physics lab's sea (#138): the island's waves, open, no shore.
            LabScene::Sea if args.water() => Some(super::water(ctx)?),
            // The flood's water (#144): a pool drawn from its columns, no sea; the cascades
            // still carry its ripples.
            LabScene::Flood if args.water() => {
                let water = super::water(ctx)?;
                water.1.set_sea(false);
                Some(water)
            }
            _ => None,
        })
    }

    fn key_pressed(&mut self, code: KeyCode, camera: &FlyCamera) -> bool {
        let lab = &mut self.lab;
        match code {
            // Space: the player jumps in the playground (#139), the car's handbrake on the
            // track (#140, held: see `update`), what a scene holds back let go (the wrecking
            // ball #142, the flood's gate, the tank's gate or shutter #156: `held`); elsewhere
            // it throws, as X does.
            KeyCode::Space => {
                if lab.has_player() {
                    lab.jump();
                } else if lab.held() {
                    lab.release();
                } else if !lab.has_car() {
                    lab.throw(camera.position, camera.forward());
                }
            }
            KeyCode::KeyX => lab.throw(camera.position, camera.forward()),
            // The scene starts over (the app's liquid too).
            KeyCode::Enter => lab.reset(),
            KeyCode::KeyC => self.chase = !self.chase,
            _ => return false,
        }
        true
    }

    fn update(
        &mut self,
        args: &Args,
        input: &Input,
        camera: &FlyCamera,
        frame: u64,
        dt: f32,
        step: f32,
    ) -> Option<f64> {
        let mut sea_time = None;
        {
            let lab = &mut self.lab;
            if let Some(every) = self.flags.throw_every
                && frame > 0
                && frame.is_multiple_of(every.max(1))
            {
                lab.throw(camera.position, camera.forward());
            }
            if let Some(every) = self.flags.shoot_every
                && frame > 0
                && frame.is_multiple_of(every.max(1))
            {
                lab.shoot(camera.position, camera.forward());
            }
            if self.flags.release == Some(frame) {
                lab.release();
            }
            // The creatures (#143): ↓ lets their motors go, ↑ powers them again; or
            // `--limp-at N`.
            if let Some(limp) = lab.creatures() {
                let wanted =
                    if self.flags.limp_at == Some(frame) || input.is_down(KeyCode::ArrowDown) {
                        true
                    } else if input.is_down(KeyCode::ArrowUp) {
                        false
                    } else {
                        limp
                    };
                if wanted != limp {
                    lab.limp(wanted);
                }
            }
            // The boat's motor (#138): the arrows, or `--steer` from the first frame; a command
            // when they change.
            let keys = |a: KeyCode, b: KeyCode| {
                f32::from(u8::from(input.is_down(a))) - f32::from(u8::from(input.is_down(b)))
            };
            let mut steering = (
                keys(KeyCode::ArrowUp, KeyCode::ArrowDown),
                keys(KeyCode::ArrowRight, KeyCode::ArrowLeft),
            );
            if let Some(s) = &self.flags.steer {
                steering = (s[0], s.get(1).copied().unwrap_or(0.0));
            }
            if steering != self.steering {
                self.steering = steering;
                lab.steer(steering.0, steering.1);
            }
            // The aeroplane (#141): W and S open and close the throttle, the arrows are the
            // stick (down pulls the nose up, left and right roll; half the elevator, all of it
            // with Shift, as a full pull from the keys stalls it), A and D the rudder; or
            // `--pilot T,E,A,R`. A command when they change; the camera follows it. The rocket
            // (#148) takes the same: the stick swings its engine, the ailerons are its roll jets.
            if lab.has_plane() || lab.has_rocket() || lab.has_ship() {
                self.chase = true;
                let keys = |a: KeyCode, b: KeyCode| {
                    f32::from(u8::from(input.is_down(a))) - f32::from(u8::from(input.is_down(b)))
                };
                let throttle = (self.flying[0] + 0.5 * step * keys(KeyCode::KeyW, KeyCode::KeyS))
                    .clamp(0.0, 1.0);
                let shift = input.is_down(KeyCode::ShiftLeft) || input.is_down(KeyCode::ShiftRight);
                let elevator = if shift { 1.0 } else { 0.5 };
                let mut flying = [
                    throttle,
                    elevator * keys(KeyCode::ArrowUp, KeyCode::ArrowDown),
                    keys(KeyCode::ArrowRight, KeyCode::ArrowLeft),
                    keys(KeyCode::KeyD, KeyCode::KeyA),
                ];
                if let Some(f) = &self.flags.pilot {
                    flying = std::array::from_fn(|k| f.get(k).copied().unwrap_or(0.0));
                }
                if flying != self.flying {
                    self.flying = flying;
                    lab.fly(flying);
                }
            }
            // The car's handbrake (#140): Space held.
            if lab.has_car() {
                let pulled = input.is_down(KeyCode::Space);
                if pulled != self.handbrake {
                    self.handbrake = pulled;
                    lab.handbrake(pulled);
                }
            }
            // The tug-of-war (#149): ← held pulls with all the left team's strength, → held
            // eases to a fifth, neither holds at half.
            if lab.has_tug() {
                let pulling = if input.is_down(KeyCode::ArrowLeft) {
                    1.0
                } else if input.is_down(KeyCode::ArrowRight) {
                    0.2
                } else {
                    0.5
                };
                if pulling != self.pulling {
                    self.pulling = pulling;
                    lab.pull(pulling);
                }
            }
            // The mission's record read again when its file changes (#287).
            lab.poll_mission();
            // The playground's player (#139): a command when the walk changes.
            if lab.has_player() {
                let walk = if self.flags.mission_autopilot {
                    autopilot(lab.mission_target())
                } else {
                    city_blocks::wished_walk(args, camera.yaw, input)
                };
                if walk != self.walking {
                    self.walking = walk;
                    lab.walk(walk);
                }
            }
            // Behind the game's menu (#352) the step is 0: a fixed step holds too.
            lab.advance(dt, args.fixed_step && step > 0.0);
            // Its player's feet as drawn this frame, for its footsteps (#318).
            self.feet = lab.player().map(|p| p.position);
            if let Some(time) = lab.sea_time() {
                sea_time = Some(time);
            }
        }
        sea_time
    }

    fn player(&mut self) -> Option<Vec3> {
        self.lab.player().map(|p| p.position)
    }

    fn footing(&self) -> Option<(Vec3, forge_audio::Surface)> {
        let (_, surface) = self.lab.footing()?;
        Some((self.feet?, surface))
    }

    fn networked(&self) -> bool {
        self.flags.net.is_some()
    }

    fn sounds(&mut self, sound: &mut forge_app::sound::Sound, _listener: Vec3) {
        // The contacts are kept from the first frame that has sound to play them.
        let impacts = self.impacts.get_or_insert_with(|| {
            self.lab.hear();
            sounds::Impacts::new(sound)
        });
        self.lab.take_heard(&mut self.heard);
        impacts.play(sound, &self.heard);
    }

    fn follow(&mut self, camera: &mut FlyCamera) {
        // C: the camera behind the lab's boat, car or aeroplane and over it, looking where it
        // goes (#138, #140, #141); further back from the aeroplane, 7 m long with a 10 m span.
        let (back, over, pitch) = if self.lab.has_plane() {
            (17.0, 3.5, -0.1)
        } else if self.lab.has_birds() {
            // A gull (#184), 1.3 m across, from 3 m behind and a little over it.
            (3.0, 0.7, -0.12)
        } else {
            (8.0, 2.8, -0.18)
        };
        let rocket = self.lab.has_rocket();
        let ship = self.lab.has_ship();
        if self.chase
            && ship
            && let Some(ride) = self.lab.ride()
        {
            // The spaceship from behind and over it, along its nose and its up as it turns,
            // looking a little ahead of it.
            let forward = ride.rotation * Vec3::NEG_Z;
            let up = ride.rotation * Vec3::Y;
            camera.position = ride.position - forward * 24.0 + up * 6.0;
            let to = ride.position + forward * 10.0 - camera.position;
            camera.yaw = (-to.x).atan2(-to.z);
            camera.pitch = to.y.atan2(Vec3::new(to.x, 0.0, to.z).length());
        } else if self.chase
            && rocket
            && let Some(ride) = self.lab.ride()
        {
            // The rocket (#148) from 30 m off its right, a little behind and over its middle,
            // looking at it: its pitch downrange (−z) crosses the view.
            let middle = ride.position + ride.rotation * Vec3::new(0.0, 6.0, 0.0);
            camera.position = middle + Vec3::new(30.0, 2.0, 5.0);
            let to = middle - camera.position;
            camera.yaw = (-to.x).atan2(-to.z);
            camera.pitch = to.y.atan2(Vec3::new(to.x, 0.0, to.z).length());
        } else if self.chase
            && let Some(ride) = self.lab.ride()
        {
            let forward = ride.rotation * Vec3::NEG_Z;
            let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
            camera.position = ride.position - flat * back + Vec3::new(0.0, over, 0.0);
            camera.yaw = (-flat.x).atan2(-flat.z);
            camera.pitch = pitch;
        }
    }

    fn movers(&mut self, scene: &mut MeshletScene) {
        // #338's check: an instance added 1 km under the ground, then removed, while the movers
        // and the skinned creatures run; the frames stay the scene built whole's.
        // `=promote` and `=swing` (#338 piece 5): the instance promoted to a mover, which takes
        // it where `=visible`'s edit moves it, then demoted there.
        let moved = (
            self.edit_at + glam::DVec3::new(2.5, 0.0, 0.0),
            glam::Quat::from_rotation_y(0.5),
        );
        if let (Some(editor), Some(mesh)) = (self.editor.as_mut(), self.lab.edited) {
            self.edit_frames += 1;
            let edit = match self.edit_frames {
                EDIT_PROMOTE if self.promote.is_some() => {
                    let slot = self.edit_slot.expect("the check's instance");
                    self.promoted = Some(editor.promote(slot).expect("a free mover"));
                    true
                }
                EDIT_MOVE if self.promote.is_some() => false,
                EDIT_DEMOTE if self.promote.is_some() => {
                    let mover = self.promoted.take().expect("the promoted mover");
                    editor.demote(
                        mover,
                        forge_render::CellPos::from_f64(moved.0),
                        moved.1,
                        1.0,
                    );
                    true
                }
                EDIT_ADD => {
                    let slot = editor
                        .add_instance_at(
                            mesh,
                            forge_render::CellPos::from_f64(self.edit_at),
                            glam::Quat::IDENTITY,
                            1.0,
                            forge_core::material::MaterialId(0),
                        )
                        .expect("room for the check's instance");
                    self.edit_slot = Some(slot);
                    true
                }
                EDIT_MOVE => {
                    // Moved and its visibility set again in place (#338 step 3).
                    if let Some(slot) = self.edit_slot {
                        editor.set_instance_transform(
                            slot,
                            forge_render::CellPos::from_f64(
                                self.edit_at + glam::DVec3::new(2.5, 0.0, 0.0),
                            ),
                            glam::Quat::from_rotation_y(0.5),
                            1.0,
                        );
                        editor.set_instance_visibility(slot, forge_render::Visibility::ALL);
                    }
                    true
                }
                EDIT_REMOVE => {
                    if let Some(slot) = self.edit_slot.take() {
                        editor.remove_instance(slot);
                    }
                    true
                }
                _ => false,
            };
            if edit {
                let edit = editor.finish(None).expect("the check's edit");
                scene.apply(edit).expect("the check's edit applied");
                tracing::info!(
                    frame = self.edit_frames,
                    "dynamic scene edited (#338's check)"
                );
            }
        }
        // `=cells` (#338 step 4): the viewer crosses the view along x, 100 m west of the pyramid
        // to 80 m east from frame 30 to 330; each frame takes every edit its position asks for
        // (`update_settled`), so a run's frames do not hang on the worker's timing.
        if let Some((streamer, device)) = self.cells.as_mut() {
            self.edit_frames += 1;
            let f = self.edit_frames as f64;
            let viewer =
                glam::DVec3::new((-100.0 + 0.6 * (f - 30.0)).clamp(-100.0, 80.0), 0.0, 0.0);
            // Facing its way east: the cells ahead load first (#345).
            let seq = streamer.view_facing(viewer, Some(glam::DVec3::X));
            let mut lamps = forge_render::lamps::LampTable::new(0);
            let mut systems = forge_render::CellSystems {
                probes: None,
                glass: None,
                lamps: &mut lamps,
            };
            // `FORGE_CELL_REALTIME`: as a game takes them, an edit a frame at most, unwaited.
            let taken = std::time::Instant::now();
            let took = if std::env::var_os("FORGE_CELL_REALTIME").is_some() {
                streamer.update(device, scene, &mut systems, 1, None)
            } else {
                streamer.update_settled(seq, device, scene, &mut systems)
            };
            match took {
                Ok(took) if took.edits > 0 => tracing::info!(
                    frame = self.edit_frames,
                    x = viewer.x,
                    edits = took.edits,
                    loaded = took.loaded,
                    unloaded = took.unloaded,
                    swapped = took.swapped,
                    cells = streamer.loaded(),
                    proxies = streamer.proxies(),
                    worker_ms = %format_args!("{:.1}", took.worker_s * 1e3),
                    frame_ms = %format_args!("{:.2}", taken.elapsed().as_secs_f64() * 1e3),
                    "cells streamed (#338's check)"
                ),
                Ok(_) => {}
                Err(e) => tracing::error!("#338's cell stream: {e}"),
            }
        }
        // The promoted mover's pose, before the movers are committed: where the instance stood,
        // then where `=visible`'s edit moves it on the frame that edit is published (no motion of
        // its own, as the edit's), or swung there over the frames to its demotion (`=swing`).
        if let (Some(mover), Some(swing)) = (self.promoted, self.promote) {
            let f = self.edit_frames;
            let t = if swing {
                // There the frame before the demotion is made.
                ((f - EDIT_PROMOTE) as f32 / (EDIT_DEMOTE - 1 - EDIT_PROMOTE) as f32).min(1.0)
            } else if f > EDIT_MOVE {
                1.0
            } else {
                0.0
            };
            let at = self.edit_at.lerp(moved.0, f64::from(t));
            let transform = forge_render::meshlet::MoverTransform {
                position: forge_render::CellPos::from_f64(at).relative_to(scene.origin()),
                rotation: glam::Quat::IDENTITY.slerp(moved.1, t),
                scale: 1.0,
            };
            scene.set_movers_at(mover, &[transform], |_| !swing);
            if f == EDIT_PROMOTE + 1 {
                tracing::info!(
                    frame = f,
                    shown = scene.mover_shown(mover),
                    "promoted (#338's check)"
                );
            }
        }
        {
            let lab = &self.lab;
            let (mut skins, mut morphs) = (Vec::new(), Vec::new());
            scene.set_movers(&lab.movers(&mut skins, &mut morphs));
            // The skinned creatures' joints (#165), and their morph targets' weights (#169).
            scene.set_skins(&skins);
            if !morphs.is_empty() {
                scene.set_morphs(&morphs);
            }
            // The yard's beds' heights (#185) when they changed, and once more after (#186).
            let mut heights = Vec::new();
            let changed = lab.fields(self.beds_seen, &mut heights);
            if heights.is_empty() && self.beds_again {
                lab.fields(changed.wrapping_add(1), &mut heights);
                self.beds_again = false;
            } else if !heights.is_empty() {
                self.beds_again = true;
            }
            if !heights.is_empty() {
                scene.set_fields(&heights);
            }
            self.beds_seen = changed;
        }
    }

    fn liquid_steps(&mut self) -> (Vec<LiquidStep>, u64) {
        let steps = self.lab.take_tank_steps();
        (steps, self.lab.now())
    }

    fn columns(&mut self) -> Option<(u64, &forge_physics::shallow::Pool)> {
        self.lab.columns()
    }

    fn pool(&mut self) -> Option<PoolView> {
        self.lab.pool()
    }

    fn splashes(&mut self, out: &mut Vec<SplashSource>) {
        // The flood's front and its water striking the walls (#162), the yard's slipping
        // wheels' spray (#192).
        self.lab.splashes(out);
    }

    fn particles(
        &mut self,
        time: f32,
        particles: &forge_render::Particles,
        emitters: &mut Vec<forge_render::Emitter>,
        bursts: &mut Vec<forge_render::Burst>,
    ) {
        // The smoke yard's chimneys, fire and footsteps (D-064).
        if self.kind == LabScene::Smoke {
            super::smoke::particles(time, particles, emitters, bursts);
        }
    }

    fn lamps(&mut self, time: f32) -> Vec<forge_render::lamps::Lamp> {
        // The room at night's three lamps and the smoke yard's fire (D-065); the Bistro's glowing
        // pieces at night (D-087).
        match self.kind {
            LabScene::Lamps => super::room::lamps(),
            LabScene::Smoke => super::smoke::lamps(time),
            _ => self
                .kind
                .authored()
                .map_or_else(Vec::new, |a| super::bistro::lamps(a, self.bistro_lamps)),
        }
    }

    fn title(&mut self) -> String {
        self.lab.title()
    }
}

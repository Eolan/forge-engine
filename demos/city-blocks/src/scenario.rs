//! A demo's own scenes in the shared app (#216, D-053): the app draws them and runs their frames,
//! with the city's renderer, its sky, its water, its liquid and its anti-aliasing; a scene says
//! what it shows, how the sun lights it, where the camera starts, and drives its moving parts.
//! `physics-lab`'s scenes are one. The city and the island are the app's own.
//!
//! - [`Scenario`] sets the scene up: its props to cook, its look, its liquid, its start camera,
//!   and its scene built on the device ([`Scenario::build`]).
//! - [`Running`] is the built scene, frame by frame: the keys, the update, the camera it
//!   follows, the movers it draws, and what it hands the app's water, liquid and spray.

use anyhow::Result;
use forge_app::{FlyCamera, Input, Setup};
use forge_geom::city::PropSpec;
use forge_procgen::Ocean;
use forge_render::{
    LiquidLook, LiquidMode, LiquidSolver, LiquidStep, LiquidTank, MeshletScene, SplashSource,
    WaterCascades, WaterSurface,
};
use glam::Vec3;
use winit::keyboard::KeyCode;

use crate::{Args, Cooked};

/// What surrounds a scene where nothing is drawn, and what lights its shaded sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Backdrop {
    /// The ground's sky, its clouds and its night, and its light.
    Sky,
    /// Space's sky (`SpaceSky`): the stars, the sun's disc and a planet; no air, no clouds, no
    /// night; the shaded sides lit by a constant fill.
    Space,
    /// Indoors: the ground's sky through the openings, no clouds, no night; a constant fill.
    Indoors,
    /// A card of this colour (its reflectance, lit by the sun) behind the scene and under the
    /// liquid, no sky; a constant fill.
    Card(Vec3),
}

/// How a scene is lit and shown.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    /// The sun's direction (towards it, normalized by the app), when the scene sets it.
    pub sun: Option<Vec3>,
    /// The sun white, as no air reddens it.
    pub white_sun: bool,
    /// What surrounds the scene.
    pub backdrop: Backdrop,
    /// The spray of the scene's own splashes ([`Running::splashes`]), with or without water.
    pub splashes: bool,
    /// The scene's particles ([`Running::particles`], D-064): smoke, dust, embers.
    pub particles: bool,
    /// The probes' rays, their finest spacing (metres) and their cascades, when the scene sets
    /// them (unless the flags do).
    pub probes: Option<(u32, f32, u32)>,
    /// The probes' rays meet the scene's movers (`ProbeParams::movers`, #299): a scene whose
    /// movers are what it shows (the physics lab's bodies). Off: walkers and vehicles stay out of
    /// the probes' light, and the probes do not wait for their upload.
    pub probes_see_movers: bool,
    /// The open sky's share a far room without its own probes takes (#321,
    /// `ProbeParams::far_room_sky`), when the scene sets it (unless `--probe-far-sky` does): its
    /// rooms' daylight factor (0.06 the village's; shallow rooms with wide windows about 0.27).
    pub probe_far_room_sky: Option<f32>,
    /// The camera follows the scene's vehicle from the start ([`Running::follow`]).
    pub chase: bool,
    /// The exposure follows the scene's metered light without `--day` or `--time-of-day` (unless
    /// `--fixed-exposure`): a map walked into from the street, whose rooms, lit through their
    /// doors and windows, are a few stops under the street, as the eye adapts to them.
    pub metered: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            sun: None,
            white_sun: false,
            backdrop: Backdrop::Sky,
            splashes: false,
            particles: false,
            probes: None,
            probes_see_movers: false,
            probe_far_room_sky: None,
            chase: false,
            metered: false,
        }
    }
}

/// A liquid in a glass tank (#156) the app simulates and draws on the GPU.
#[derive(Clone, Debug)]
pub struct LiquidSetup {
    /// The tank and the water in it.
    pub tank: LiquidTank,
    /// How it is solved.
    pub solver: LiquidSolver,
    /// How it is drawn.
    pub look: LiquidLook,
    /// What its drawing shows at first (keys 1, 2 and 3 change it).
    pub mode: LiquidMode,
    /// The tank's inside's corner in the scene, metres.
    pub corner: Vec3,
    /// The scene's ticks between the liquid's lines in the log.
    pub log_every: u64,
}

/// A scene's water: the cascades of FFT waves, the surface that draws them, and the spectra
/// they came from (the app's start-up check).
pub type Water = (WaterCascades, WaterSurface, Vec<Ocean>);

/// A pool of water a scene holds as samples of its surface (#144): the app's water draws it.
#[derive(Clone, Debug)]
pub struct PoolView {
    /// Its first sample's corner, metres (x, z).
    pub origin: [f32; 2],
    /// Metres between samples.
    pub spacing: f32,
    /// Samples a side.
    pub size: [u32; 2],
    /// Each sample's level, flow and depth, as `forge_render::WaterPool` takes them.
    pub samples: Vec<[f32; 4]>,
}

/// A demo's scene, before it is built: what the app needs to know of it up front.
pub trait Scenario: Send {
    /// The props to cook (or load), behind the loading screen.
    fn props(&self) -> Vec<PropSpec>;
    /// How it is lit and shown.
    fn look(&self) -> Look;
    /// Its liquid, if it has one.
    fn liquid(&self) -> Option<LiquidSetup>;
    /// Where the camera starts (the app applies `--view` and `--fov` over it).
    fn camera(&self) -> FlyCamera;
    /// Its CPU work before the build (its meshes cooked or read from the cache), on the loading
    /// thread: beside a fresh cache's shaders compiled ahead, where the build waits for them
    /// (#274). Nothing by default.
    fn prepare(&mut self) {}
    /// Builds it on the device from the cooked props.
    fn build(
        self: Box<Self>,
        ctx: &Setup,
        args: &Args,
        cooked: Cooked,
    ) -> Result<(MeshletScene, Box<dyn Running>)>;
}

/// A built scene, frame by frame.
pub trait Running: Send {
    /// Its water, made once after the build (none by default).
    fn water(&mut self, _ctx: &Setup, _args: &Args) -> Result<Option<Water>> {
        Ok(None)
    }
    /// Its rooms for the probes (D-068, [`forge_render::Probes::set_regions`]), in the world
    /// frame: x, y up and z in metres from the world's origin. None by default: every probe
    /// lights every surface.
    fn probe_regions(&self) -> Vec<forge_render::RegionVolume> {
        Vec::new()
    }
    /// A key pressed: whether the scene took it. An Enter it takes starts the app's liquid over
    /// too.
    fn key_pressed(&mut self, code: KeyCode, camera: &FlyCamera) -> bool;
    /// Its update before the camera's, at the app's `frame` (`dt` the frame's seconds, `step`
    /// what the scripted clocks advance by): returns the water's time when the scene sets it.
    fn update(
        &mut self,
        args: &Args,
        input: &Input,
        camera: &FlyCamera,
        frame: u64,
        dt: f32,
        step: f32,
    ) -> Option<f64>;
    /// Where its player's feet are, for the camera to follow (the right mouse button turns it).
    fn player(&mut self) -> Option<Vec3>;
    /// After the camera's update: the camera behind the vehicle it follows, if it follows one.
    fn follow(&mut self, camera: &mut FlyCamera);
    /// Its movers, skins, morph targets and fields into the scene, before the frame's draw.
    fn movers(&mut self, scene: &mut MeshletScene);
    /// The liquid's substeps the ticks since the last frame owe it, and the scene's tick.
    fn liquid_steps(&mut self) -> (Vec<LiquidStep>, u64) {
        (Vec::new(), 0)
    }
    /// Its shallow water's columns at its tick, for the GPU's finer layer (#162).
    fn columns(&mut self) -> Option<(u64, &forge_physics::shallow::Pool)> {
        None
    }
    /// Its pool of water as samples, when the GPU's layer does not draw it.
    fn pool(&mut self) -> Option<PoolView> {
        None
    }
    /// Its splashes' sources this frame.
    fn splashes(&mut self, _out: &mut Vec<SplashSource>) {}
    /// Its lamps at `time` (the particles' clock, seconds; D-065), in the world frame (as
    /// `probe_regions`). None by default.
    fn lamps(&mut self, _time: f32) -> Vec<forge_render::lamps::Lamp> {
        Vec::new()
    }
    /// Its clear glass over the rooms it builds (D-065's part 4), taken once after the build, in
    /// the world frame (as `probe_regions`): panes its meshes leave out. None by default.
    fn glass(&self) -> forge_render::GlassPanes {
        forge_render::GlassPanes::default()
    }
    /// Its clear panes' edits since the last frame at `time` (the particles' clock, seconds;
    /// #325), in the world frame: each pane's id ([`forge_render::GlassPanes::pane`]) and its
    /// transform from where it was given (a sash raised), or `None` to hide it (a window
    /// smashed). None by default.
    fn glass_edits(&mut self, _time: f32) -> Vec<(u32, Option<glam::Mat4>)> {
        Vec::new()
    }
    /// Its particles' emitters and bursts at `time` (the splashes' clock, seconds), in the
    /// scene's frame (D-064): the effects by name through `particles`.
    fn particles(
        &mut self,
        _time: f32,
        _particles: &forge_render::Particles,
        _emitters: &mut Vec<forge_render::Emitter>,
        _bursts: &mut Vec<forge_render::Burst>,
    ) {
    }
    /// What the window's title adds.
    fn title(&mut self) -> String;
    /// Where its player's feet are while it walks in first person (the camera at its eyes) and
    /// what they walk on, for their footsteps (#318), in the scene's frame. None by default.
    fn footing(&self) -> Option<(Vec3, forge_audio::Surface)> {
        None
    }
    /// Its own sounds this frame (#318), in the scene's frame, heard at `listener` (the
    /// camera); `sound` is the app's output, there only in interactive runs. None by default.
    fn sounds(&mut self, _sound: &mut forge_app::sound::Sound, _listener: Vec3) {}
}

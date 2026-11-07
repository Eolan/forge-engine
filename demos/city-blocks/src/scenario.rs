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
    Tonemap, WaterCascades, WaterSurface,
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
    /// The sun's colour as the scene's authors set it (linear RGB, its hue alone: the light's
    /// level stays the sunlight's through the air at its height). None: the air's. `white_sun`
    /// wins over it, and `--day`, `--time-of-day` or an environment map's sun, which colour the
    /// sun themselves, leave it unused.
    pub sun_color: Option<Vec3>,
    /// The sun's illuminance at the ground, lux (the sky's light with it), the app's where none
    /// (128,000): an authored world's lamps are set against its own sun, and dimming its sun
    /// keeps them as their data gives them (their reach, and so their cost, unchanged). `--day`
    /// and `--time-of-day` under the ground's own sky give the app's; an environment map or
    /// another backdrop keeps this.
    pub sun_illuminance: Option<f32>,
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
    /// rooms' daylight factor (0.2 the village's; shallow rooms with wide windows about 0.27).
    pub probe_far_room_sky: Option<f32>,
    /// The far rooms' daylight (#356, `ProbeParams::far_room_daylight`), when the scene sets it
    /// (unless `--probe-far-daylight` does): its rooms then take their windows' share of the
    /// open sky, following the hour, and `probe_far_room_sky` is left to rooms without windows.
    pub probe_far_room_daylight: Option<f32>,
    /// The far rooms' lamps (#363, `ProbeParams::far_room_lamps`), when the scene sets it (unless
    /// `--probe-far-lamps` does): with their daylight, its far rooms take their lamps' light round
    /// their walls.
    pub probe_far_room_lamps: Option<[f32; 2]>,
    /// The rooms' own probe grids (D-078, `ProbeParams::rooms`), when the scene sets them (unless
    /// `--probe-rooms` and `--probe-room-spacing` do): their reach, spacing and pool. Set, they are
    /// on even where the scene gives no rooms at the start (a map streaming its rooms in, #338).
    /// A town's street of houses may want a shorter reach than the default 20 m: each room within
    /// it updates its grid every frame (a game measured +0.41 ms at 20 m, +0.18 at 12 m on a
    /// street walk).
    pub probe_room_grids: Option<forge_render::RoomGridParams>,
    /// The whole scene is indoors (a room authored as one), so under `r.gi.cache auto` SHaRC
    /// lights every bounce's hit (D-090); else only in the probes' room grids' rooms.
    pub indoors: bool,
    /// The camera follows the scene's vehicle from the start ([`Running::follow`]).
    pub chase: bool,
    /// The exposure follows the scene's metered light without `--day` or `--time-of-day` (unless
    /// `--fixed-exposure`): a map walked into from the street, whose rooms, lit through their
    /// doors and windows, are a few stops under the street, as the eye adapts to them.
    pub metered: bool,
    /// The metered exposure's compensation where `--exposure-compensation` gives none, by day (a
    /// run that starts at night keeps the shared stop): an authored scene keyed as its authors'
    /// pictures are (D-089).
    pub exposure_compensation: Option<f32>,
    /// The metered exposure's lowest and highest EV100 (`AutoExposure::range`), the app's where
    /// none (or where it is not one: inverted, not finite): an authored scene's own limits, so a
    /// dark room is shown as much darker than the street as its authors' pictures show it. By day
    /// and by night alike (the night's floor, D-046, lifts nothing under it).
    pub exposure_range: Option<(f32, f32)>,
    /// Local exposure (D-067) on from the start (unless `--no-local-exposure`), with the meter
    /// keyed on a lower band (0.25–0.95) and the shadows' contrast at 0.55 (unless
    /// `--meter-band`, `--local-exposure-contrast`): a room seen from a lit hall through its door
    /// reads a bit darker, as the eye sees it, not black (the owner, 2026-10-19, #275).
    pub local_exposure: bool,
    /// With `metered`: by day, outdoors, how far the exposure follows a scene darker than a
    /// sunlit street (`AutoExposure::shade_adapt`, `r.exposure.shade_adapt`); none, all the way.
    pub shade_adapt: Option<f32>,
    /// With `metered`: how fast the exposure darkens and brightens the image, per second
    /// (`AutoExposure::speed_darken`, `speed_brighten`); none, their defaults.
    pub exposure_speeds: Option<(f32, f32)>,
    /// What the diffuse rays meet of a glow, times this (`MeshletRenderer::ray_glow`); none, as it is. A fit,
    /// for an authored scene lit by its neons.
    pub ray_glow: Option<f32>,
    /// The particle effects that take a share of the lamps are lit by them
    /// (`Particles::light_by_lamps`).
    pub particle_lamps: bool,
    /// A bloom of the highlights alone, added over the image: from nothing at the first pre-exposed
    /// luminance to all of it at the second (`Bloom::highlights`), and how much is added. The
    /// image keeps its sharpness where a share of a blur of all of it would soften it. None: the
    /// app's bloom.
    pub bloom_highlights: Option<[f32; 3]>,
    /// An environment map as the sky (D-084), for a scene authored under one: no air, no
    /// clouds, no night; the sun's direction from `sun`.
    pub environment: Option<Environment>,
    /// The scene's tone curve where `--tonemap` names none (AgX punchy otherwise): an authored
    /// scene's renders made under another.
    pub tonemap: Option<Tonemap>,
    /// The scene's own grading table (D-100), shown under [`Tonemap::SceneLut`]: an authored
    /// scene graded by its source's table. Held for the program's life.
    pub grading: Option<&'static forge_render::Grading>,
    /// The air's aerosols against a clear day's (D-101): its Mie scattering and absorption
    /// times this, when the scene sets it (a finite value above 0). A city's haze: a paler sky,
    /// a whitish distance and a glow round the sun, the air's blue (Rayleigh) unchanged. The
    /// sun reaches the ground through it dimmer and warmer: `sun_color` sets its hue alone, at
    /// that luminance, and `sun_illuminance` the light above the air (a scene fitted to its
    /// sun's level is fitted again). None: Earth's clear day (`AtmosphereParams::earth`).
    pub aerosols: Option<f32>,
    /// The sky's light on surfaces times this, the sky as drawn and in mirrors unchanged
    /// (`SkyParams::light_share`). None: 1.
    pub sky_light: Option<f32>,
    /// How far the sky's light on surfaces goes towards white, the sky as drawn unchanged
    /// (`SkyParams::light_white`). None: its colour.
    pub sky_white: Option<f32>,
}

/// An environment map as a scene's sky and sun (D-084): the light its authors set, for
/// comparisons with their renders. The photograph's sun, taken out of the map, is the scene's
/// sun (its direction, colour and the reference illuminance); a map without one keeps the
/// scene's.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    /// The Radiance HDR file.
    pub path: &'static str,
    /// The sky's light against its sun's: 1 as photographed. Without a sun in the map, the
    /// luminance of a unit of it per unit of the scene's sun's illuminance.
    pub scale: f32,
    /// Whether its sun (and the sky with it) is dimmed by Forge's atmosphere at the sun's height,
    /// as Forge's own sky's sun is: a photograph's level is its camera's, and its sun given the
    /// light above the atmosphere (128,000 lux) lit a street 1.4 to 1.8 times a clear day's. Off,
    /// the sun is that light, for the scenes compared with Falcor at its units.
    pub through_atmosphere: bool,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            sun: None,
            white_sun: false,
            sun_color: None,
            sun_illuminance: None,
            backdrop: Backdrop::Sky,
            splashes: false,
            particles: false,
            probes: None,
            probes_see_movers: false,
            probe_far_room_sky: None,
            probe_far_room_daylight: None,
            probe_far_room_lamps: None,
            probe_room_grids: None,
            indoors: false,
            chase: false,
            metered: false,
            exposure_compensation: None,
            exposure_range: None,
            local_exposure: false,
            shade_adapt: None,
            exposure_speeds: None,
            ray_glow: None,
            particle_lamps: false,
            bloom_highlights: None,
            environment: None,
            tonemap: None,
            grading: None,
            aerosols: None,
            sky_light: None,
            sky_white: None,
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

/// A scene's fog everywhere and how its lights scatter in it ([`Running::fog_environment`]).
#[derive(Clone, Copy, Debug)]
pub struct FogEnvironment {
    /// Its fog everywhere, its height in the world frame (none: the boxes alone).
    pub global: Option<forge_render::GlobalFog>,
    /// How strongly the sun's light, then the lamps', scatter in the fog (1: whole).
    pub light_scales: [f32; 2],
    /// How strongly the probes' light scatters in the fog everywhere, then in the boxes (1: whole;
    /// `FogParams::ambient`).
    pub ambient_scales: [f32; 2],
    /// The phase's anisotropy g for its sun's (its global light's) light; the lamps' keep the
    /// app's (`FogParams::lamp_anisotropy`).
    pub anisotropy: f32,
    /// Its distance fog beyond the froxels (D-101), its height in the world frame (none: the
    /// froxels' fog alone).
    pub distance: Option<forge_render::DistanceFog>,
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
    /// Its fog volumes (D-099, [`forge_render::FogVolume`]), in the world frame as
    /// [`Self::probe_regions`]: lit by the lamps and the sun. None by default.
    fn fog_boxes(&self) -> Vec<forge_render::FogBox> {
        Vec::new()
    }
    /// Its lamps that light only its fog (D-099), in the world frame: an authored scene's lights
    /// for its fog alone. None by default.
    fn fog_lamps(&self) -> Vec<forge_render::lamps::Lamp> {
        Vec::new()
    }
    /// Its fog everywhere and how its lights scatter in it (D-099): an authored scene's
    /// environment, its height in the world frame. None by default: the boxes alone, the sun and
    /// the lamps whole.
    fn fog_environment(&self) -> Option<FogEnvironment> {
        None
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
    /// Its pictures of light with no surface, taken once after the build, in the world frame (as
    /// `glass`): pictures its meshes leave out. None by default.
    fn holograms(&self) -> forge_render::HologramPanes {
        forge_render::HologramPanes::default()
    }
    /// Its grass near the camera (#300, D-075), taken once after the build, in the world frame
    /// (as `glass`): its map, and the material row of its blades. None by default.
    fn grass(&self) -> Option<(forge_render::grass::GrassMap, u32)> {
        None
    }
    /// Its impostors (D-075 step 4): the meshes to bake, each with a name for its atlases'
    /// files, and the plants drawn with them, taken once after the build. None by default.
    fn impostors(&mut self) -> Option<forge_render::impostor::ImpostorSet> {
        None
    }
    /// Its doorways shut or opened by their doors since the last frame at `time` (the
    /// particles' clock, seconds; #327): each its place in [`Running::probe_regions`] and the
    /// share of the way across from its `region` side its door's leaf stands at, or `None`
    /// open ([`forge_render::Probes::set_doorway`]). None by default.
    fn doorway_edits(&mut self, _time: f32) -> Vec<(usize, Option<f32>)> {
        Vec::new()
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
    /// Its cells streamed round the camera (D-096, #338's `CellStreamer`), for a scene built to be
    /// edited (`MeshletSceneBuilder::reserve_dynamic`): the app sends the streamer the camera's
    /// place in the scene frame each frame (the partition's frame must be the scene's) and applies
    /// its edits, every change the camera asks for settled before the frame (a capture's runs, as
    /// the physics lab's), or one edit a frame unwaited as a game takes them with
    /// `FORGE_CELLS_REALTIME=1`. The cells' sets (rooms, panes, lamps) are not taken yet: give the
    /// lamps through [`Running::lamps`]. None by default.
    fn cells(&mut self) -> Option<&mut forge_render::CellStreamer> {
        None
    }
    /// Where its player's feet are drawn this frame (between its ticks) while it walks in first
    /// person (the camera at its eyes), and what they walk on, for their footsteps (#318), in the
    /// scene's frame. A tick's place would stand still on the frames between ticks, which the
    /// steps read as standing. None by default.
    fn footing(&self) -> Option<(Vec3, forge_audio::Surface)> {
        None
    }
    /// Its own sounds this frame (#318), in the scene's frame, heard at `listener` (the
    /// camera); `sound` is the app's output, there only in interactive runs. None by default.
    fn sounds(&mut self, _sound: &mut forge_app::sound::Sound, _listener: Vec3) {}
    /// The place round `listener` (the camera) for the ambience (#350): how green it is within
    /// earshot, and how far indoors. None by default: open country, the beds as they were made.
    fn habitat(&self, _listener: Vec3) -> Option<forge_audio::ambience::Habitat> {
        None
    }
    /// Its birds' species and shares (#350), read once when the ambience starts. None by
    /// default: [`forge_audio::ambience::BirdMix`]'s Mediterranean village.
    fn bird_mix(&self) -> Option<forge_audio::ambience::BirdMix> {
        None
    }
    /// Where its doves perch (#357), in the scene's frame, read once when the ambience starts:
    /// each calls now and then from there ([`forge_audio::ambience::perched`]), and the birds' bed
    /// leaves the dove out. None by default: the bed's dove.
    fn dove_perches(&self) -> Vec<Vec3> {
        Vec::new()
    }
    /// Whether it runs with others, through a server: the game's menu (#352) then leaves it
    /// running. False by default: a solo scene holds behind the menu.
    fn networked(&self) -> bool {
        false
    }
    /// The lines it shows on the HUD, at the window's top left (#287: a mission's objective).
    /// None by default.
    fn hud_lines(&self) -> Vec<String> {
        Vec::new()
    }
}

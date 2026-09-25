//! `asteroids` — the ballad. A scripted flight through a dense asteroid field, the engine's
//! living showcase: whatever the renderer can do at this point, it does here, with profiling.
//!
//! Phase 0: several procedural asteroid meshes, thousands of instances, task/mesh shaders,
//! two-pass occlusion culling, a procedural starfield, a spline camera path.
//! Phase 1: cluster LOD DAG, visibility buffer, render graph, physical light units with
//! automatic exposure and a choice of tone curves, the planet under a Hillaire atmosphere.
//!
//! Controls: P pause/resume the path (right mouse look + WASD to fly freely while paused),
//! T temporal anti-aliasing, M meshlet colours, O occlusion, R software rasteriser, H show
//! what it drew, Tab wireframe, G tone curve, - / = exposure compensation, Esc quit.

#![forbid(unsafe_code)]

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_app::{AppConfig, Context, Demo, Finish, FlyCamera, FrameInfo, Input, vk};
use forge_core::hash::hash_cell3;
use forge_core::material::Material;
use forge_core::{MaterialTable, Seed, SplitMix64};
use forge_geom::{CookOptions, MeshletMesh, procedural};
use forge_render::SwRaster;
use forge_render::material::{TextureSet, stock};
use forge_render::meshlet::DrawParams;
use forge_render::{
    AmbientLight, Atmosphere, AtmosphereParams, AutoExposure, Bloom, CullCamera, CullFlags,
    Display, DlssMode, DlssUpscaler, DustParams, DustVolume, FrameStats, Gtao, GtaoParams,
    HDR_FORMAT, LuminanceMeter, MeshletRenderer, MeshletScene, MeshletSceneBuilder, Starfield, Taa,
    Tonemap, UpscaleCamera,
};
use forge_task::TaskPool;
use glam::{Mat4, Quat, Vec3};
use winit::keyboard::KeyCode;

#[derive(Parser, Debug, Clone)]
#[command(about = "The asteroid ballad")]
struct Args {
    /// Draw without the sun's ray-traced shadows between the rocks (J toggles them).
    #[arg(long)]
    no_shadows: bool,
    /// The Phase 0 rock: plain colours, no texture.
    #[arg(long)]
    no_textures: bool,
    /// The rock's texture repeating as before issue #66: no hex-tiling, one place in the texture
    /// for every rock of a shape.
    #[arg(long)]
    no_hex_tiling: bool,
    /// Leave the fill light unoccluded: no ambient occlusion (N toggles it).
    #[arg(long)]
    no_ao: bool,
    /// Soft sun shadows: the rays aim within the sun's disc (Z toggles them). Off by default: the
    /// ballad's camera never stops and its penumbrae are wide, and TAA smears their noise along
    /// the motion into streaks (issue #55).
    #[arg(long)]
    soft_shadows: bool,
    /// The Phase 0 rocks: round displaced spheres instead of fractured chunks.
    #[arg(long)]
    round_rocks: bool,
    /// The ice in the rock's shapes, not its own blockier ones (issue #63).
    #[arg(long)]
    rock_shaped_ice: bool,
    /// No weathered crust: the chunks' old surface drawn like their fracture faces (issue #62).
    #[arg(long)]
    no_crust: bool,
    /// Opaque ice: no sunlight through its thickness (Y toggles it).
    #[arg(long)]
    no_translucency: bool,
    /// One ice for every block: #59's clear ice, instead of three densities of bubbles.
    #[arg(long)]
    clear_ice: bool,
    /// Put the ice asteroids in a belt of their own along the same orbit, this many metres
    /// beyond the rock belt, away from the sun (issue #22; negative: sunward; 0: one mixed belt).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    ice_belt: f32,
    /// Draw without the sunlit dust between the rocks (V toggles it).
    #[arg(long)]
    no_dust: bool,
    /// Extinction of the densest dust, per metre (the belt's dust scatters all it takes).
    #[arg(long, default_value_t = 1e-4)]
    dust: f32,
    /// How far an occluder reaches for the ambient occlusion, metres.
    #[arg(long, default_value_t = 2.0)]
    ao_radius: f32,
    /// Bloom strength, the share of the shown image that is bloom (0 for none; B toggles it).
    #[arg(long, default_value_t = 0.04)]
    bloom: f32,
    /// Window width in pixels (the render size, before any DLSS mode scales it).
    #[arg(long, default_value_t = 1600)]
    width: u32,
    /// Window height in pixels.
    #[arg(long, default_value_t = 900)]
    height: u32,
    /// Number of asteroids in the field (3000 until issue #23).
    #[arg(long, default_value_t = 10000)]
    count: u32,
    /// Chunk shapes per size class (issue #23): each cut by its own planes, all but the first
    /// stretched; 1 is #60's seven shapes.
    #[arg(long, default_value_t = 4)]
    variants: u32,
    /// Extent of the field along its long axis (metres).
    #[arg(long, default_value_t = 1200.0)]
    length: f32,
    /// Seconds for one pass along the path.
    #[arg(long, default_value_t = 90.0)]
    duration: f32,
    /// Vertical sync.
    #[arg(long)]
    vsync: bool,
    /// Vulkan validation layer.
    #[arg(long)]
    validate: bool,
    /// Advance the path by a fixed step per frame instead of wall time (deterministic captures).
    #[arg(long)]
    fixed_step: bool,
    /// Direction to the sun, "x,y,z".
    #[arg(long, default_value = "0.75,0.30,-0.35", value_parser = parse_vec3)]
    sun_dir: Vec3,
    /// Direction to the planet, "x,y,z".
    #[arg(long, default_value = "-0.45,0.10,-1.0", value_parser = parse_vec3)]
    planet_dir: Vec3,
    /// Angular radius of the planet in degrees (0 hides it).
    #[arg(long, default_value_t = 18.0)]
    planet_angle: f32,
    /// Look in this direction, "x,y,z", instead of along the path (stills of the sky).
    #[arg(long, value_parser = parse_vec3)]
    look: Option<Vec3>,
    /// Exit after this many frames.
    #[arg(long)]
    frames: Option<u64>,
    /// Write a PNG of frame `--capture-frame` to this path.
    #[arg(long)]
    capture: Option<PathBuf>,
    /// Which frame to capture.
    #[arg(long, default_value_t = 240)]
    capture_frame: u64,
    /// Also capture every N-th frame (`<capture stem>-NNNNN.png`).
    #[arg(long)]
    capture_every: Option<u64>,
    /// Start with temporal anti-aliasing off (T toggles it).
    #[arg(long)]
    no_taa: bool,
    /// Anti-aliasing and upscaling: taa, or a DLSS mode (dlaa, quality, balanced, performance,
    /// ultra-performance; needs `--features dlss`, the Streamline SDK and an RTX GPU). U cycles
    /// them at run time.
    #[arg(long, default_value = "taa")]
    upscaler: String,
    /// Switch the anti-aliasing as U does every N frames (tests the switch in scripted runs).
    #[arg(long)]
    cycle_upscaler: Option<u64>,
    /// Start with occlusion culling off (O toggles it).
    #[arg(long)]
    no_occlusion: bool,
    /// Start with cone culling off (C toggles it).
    #[arg(long)]
    no_cone: bool,
    /// Projected LOD error a drawn cluster may have, in pixels (0.5 = finer, 2 = coarser).
    #[arg(long, default_value_t = 1.0)]
    lod_error: f32,
    /// Draw the full-detail clusters only (no LOD selection; L toggles it).
    #[arg(long)]
    no_lod: bool,
    /// Start with clusters coloured by LOD level (K toggles it).
    #[arg(long)]
    lod_colors: bool,
    /// Weight of the normals in the chunks' simplification error, in metres per unit of normal
    /// change per metre of the rock's radius (issue #65). The chunks' relief is shallow but
    /// steep: with geometry alone (0, the cooking before #65) a level flattens its shading while
    /// moving it less than a pixel, and every switch of level pops.
    #[arg(long, default_value_t = 0.5)]
    lod_normals: f32,
    /// Disable the per-group LOD window (A/B harness: must not change the image).
    #[arg(long)]
    no_group_window: bool,
    /// Share of the current frame in the temporal blend (0.1 default; 1 keeps the jitter but
    /// no history).
    #[arg(long, default_value_t = 0.1)]
    taa_blend: f32,
    /// Start with the culling-error view on (X toggles it): culled meshlets drawn in red.
    #[arg(long)]
    show_culled: bool,
    /// Force the profiling overlay on (also in scripted runs, e.g. for a capture of it).
    #[arg(long)]
    overlay: bool,
    /// Force the profiling overlay off (F1 still toggles it).
    #[arg(long)]
    no_overlay: bool,
    /// Tone curve: agx, aces or neutral (G cycles them). ACES by default here: its toe keeps
    /// space black, where AgX's wide log encoding lifts the nebula to a flat grey.
    #[arg(long, default_value = "aces")]
    tonemap: Tonemap,
    /// Fixed exposure value at ISO 100 instead of automatic exposure.
    #[arg(long)]
    ev100: Option<f32>,
    /// Exposure compensation in EV for the automatic exposure (positive = brighter; - / =).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    exposure_compensation: f32,
    /// Illuminance of the sun at the field, in lux (128 000: the Sun at 1 AU in space).
    #[arg(long, default_value_t = forge_render::starfield::SUN_ILLUMINANCE_1AU)]
    sun_lux: f32,
    /// Append "frame,path_t,ev100,target_ev100" for every frame to this CSV file.
    #[arg(long)]
    exposure_log: Option<PathBuf>,
    /// Draw through the indirect-count fallback: the device is created without mesh shaders.
    #[arg(long)]
    force_fallback: bool,
    /// When the software rasteriser draws the dense clusters: auto (when a frame holds enough
    /// of them to repay its fixed cost), on or off. R cycles them; on and off must give the
    /// same image (A/B harness).
    #[arg(long, default_value = "auto")]
    sw_raster: SwRaster,
    /// Instance occlusion (issue #38): auto (while most instances in the frustum are hidden),
    /// on or off. Every mode must give the same image (A/B harness).
    #[arg(long, default_value = "auto")]
    instance_occlusion: forge_render::InstanceOcclusion,
    /// Clusters (under 64 pixels across) whose bounding rectangle holds fewer pixels than
    /// this per triangle are rasterised in compute.
    #[arg(long, default_value_t = forge_render::meshlet::SW_RASTER_DEFAULT_AREA)]
    sw_raster_area: f32,
    /// Start with the software rasteriser's pixels tinted green (H toggles it).
    #[arg(long)]
    show_raster: bool,
}

fn parse_vec3(text: &str) -> std::result::Result<Vec3, String> {
    let parts: Vec<f32> = text
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    match parts.as_slice() {
        [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
        _ => Err("expected three numbers: x,y,z".to_owned()),
    }
}

/// A closed Catmull-Rom spline through control points.
struct Path {
    points: Vec<Vec3>,
}

impl Path {
    fn sample(&self, t: f32) -> Vec3 {
        let n = self.points.len();
        let u = t.rem_euclid(1.0) * n as f32;
        let i = u.floor() as usize;
        let f = u - i as f32;
        let p = |k: isize| self.points[(i as isize + k).rem_euclid(n as isize) as usize];
        let (p0, p1, p2, p3) = (p(-1), p(0), p(1), p(2));
        0.5 * ((2.0 * p1)
            + (-p0 + p2) * f
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * f * f
            + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * f * f * f)
    }
}

struct Ballad {
    args: Args,
    renderer: MeshletRenderer,
    starfield: Starfield,
    /// The planet's atmosphere and the camera's position relative to its centre (km).
    atmosphere: Option<(Atmosphere, Vec3)>,
    taa: Taa,
    /// Bloom before the tone curve (issue #44), on while `bloom_on`.
    bloom: Bloom,
    bloom_on: bool,
    /// Sunlit dust between the rocks (issue #58), on devices with ray queries, on while
    /// `dust_on`.
    dust: Option<DustVolume>,
    dust_on: bool,
    /// Ambient occlusion of the fill light (issue #55), on while `ao_on`.
    gtao: Gtao,
    ao_on: bool,
    taa_enabled: bool,
    /// DLSS, when the device has it; used instead of the TAA resolve while `dlss_on`.
    dlss: Option<DlssUpscaler>,
    dlss_on: bool,
    /// Takes DLSS's HDR output through the tone curve to the swapchain.
    display: Display,
    /// The size the scene is drawn at (the window's, or DLSS's input size).
    render_extent: vk::Extent2D,
    meter: LuminanceMeter,
    /// `FORGE_HASH_IMAGES=1`: per-frame image hashes in the frame trace (issue #71).
    hasher: Option<forge_render::debug_hash::ImageHasher>,
    exposure: AutoExposure,
    tonemap: Tonemap,
    /// Seconds the scene advanced this frame (the fixed step with `--fixed-step`).
    step: f32,
    exposure_log: Option<std::io::BufWriter<std::fs::File>>,
    scene: MeshletScene,
    path: Path,
    path_t: f32,
    paused: bool,
    camera: FlyCamera,
    flags: CullFlags,
    wireframe: bool,
    stats: Vec<FrameStats>,
    gpu_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    frame_ms: Vec<f64>,
    last_frame: Instant,
    title_updates: u32,
}

impl Ballad {
    fn new(ctx: &mut Context, args: Args, field: FieldMeshes) -> Result<Self> {
        // The scene is drawn into a visibility buffer and shaded into the TAA's HDR target;
        // the swapchain only receives the resolve.
        let renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, ctx.extent())?;
        let mut starfield = Starfield::new(&ctx.device, &ctx.shaders, HDR_FORMAT)?;
        // A large Earth-like planet low on the horizon, lit from the side by the sun: the
        // camera sits where its ground fills a disc of `--planet-angle`.
        let atmosphere = if args.planet_angle > 0.0 {
            let params = AtmosphereParams::earth();
            let view = params.view_from_space(args.planet_dir, args.planet_angle.to_radians());
            Some((Atmosphere::new(&ctx.device, &ctx.shaders, params)?, view))
        } else {
            None
        };
        let mut renderer = renderer;
        renderer.sun_dir = args.sun_dir.normalize_or(Vec3::Y);
        // One sun for the rocks, the planet and the disc in the sky.
        renderer.sun_illuminance = args.sun_lux;
        starfield.sun_illuminance = args.sun_lux;
        let meter = LuminanceMeter::new(&ctx.device, &ctx.shaders)?;
        let hasher = std::env::var_os("FORGE_HASH_IMAGES")
            .is_some_and(|v| v != "0")
            .then(|| forge_render::debug_hash::ImageHasher::new(&ctx.device, &ctx.shaders))
            .transpose()?;
        let exposure = match args.ev100 {
            Some(ev100) => AutoExposure::fixed(ev100),
            None => {
                let mut exposure = AutoExposure::new(15.0);
                exposure.compensation = args.exposure_compensation;
                exposure
            }
        };
        let exposure_log = args
            .exposure_log
            .as_ref()
            .map(|path| std::fs::File::create(path).map(std::io::BufWriter::new))
            .transpose()?;
        let taa = Taa::new(
            &ctx.device,
            &ctx.shaders,
            ctx.extent(),
            ctx.swapchain.format(),
        )?;
        // DLSS takes over from the TAA resolve when asked for and available; its HDR output goes
        // to the swapchain through the stand-alone display pass.
        let display = Display::new(&ctx.device, &ctx.shaders, ctx.swapchain.format())?;
        let requested = match args.upscaler.as_str() {
            "taa" => None,
            name => Some(DlssMode::from_name(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown upscaler '{name}': taa, dlaa, quality, balanced, performance or ultra-performance"
                )
            })?),
        };
        let dlss = DlssUpscaler::new(
            &ctx.device,
            requested.unwrap_or(DlssMode::Dlaa),
            ctx.extent(),
        )?;
        if requested.is_some() && dlss.is_none() {
            tracing::warn!(
                "DLSS is not available (it needs --features dlss, the Streamline SDK in streamline-sdk/ and an RTX GPU): TAA instead"
            );
        }
        let dlss_on = requested.is_some() && dlss.is_some();
        let (scene, path) = build_field(ctx, &args, field)?;
        let camera = FlyCamera {
            speed: 40.0,
            ..FlyCamera::default()
        };
        let taa_enabled = !args.no_taa;
        let mut flags = CullFlags::DEFAULT;
        if !args.no_shadows {
            flags.0 |= CullFlags::SHADOWS;
        }
        if !args.no_translucency {
            flags.0 |= CullFlags::TRANSLUCENCY;
        }
        // The chunks' cooking (issue #65) lists 120–140 k clusters a frame at 900 lines, 184 k
        // with occlusion and cone culling off (O, C), 242 k at 1440: past the list's first size.
        // Reserved up front, no frame drops any. Grown on demand instead, the first frames in
        // flight would leave clusters out, and the automatic exposure would carry their trace
        // into the frames after.
        let lines = u64::from(args.height.max(900));
        renderer.reserve_visible(u64::from(args.count) * 24 * lines / 900);
        if args.no_occlusion {
            flags.toggle(CullFlags::OCCLUSION);
        }
        if args.no_cone {
            flags.toggle(CullFlags::CONE);
        }
        if args.no_lod {
            flags.toggle(CullFlags::LOD);
            // Every frame lists at most the finest clusters: no frame has to drop any.
            renderer.reserve_visible(scene.finest_clusters);
        }
        if args.lod_colors {
            flags.toggle(CullFlags::LOD_COLORS);
        }
        if args.no_group_window {
            flags.toggle(CullFlags::GROUP_WINDOW_OFF);
        }
        if args.show_culled {
            flags.toggle(CullFlags::SHOW_CULLED);
        }
        if args.show_raster {
            flags.toggle(CullFlags::SHOW_RASTER);
        }
        let mut taa = taa;
        taa.blend = args.taa_blend;
        taa.bloom_strength = args.bloom;
        let bloom = Bloom::new(&ctx.device, &ctx.shaders)?;
        let bloom_on = args.bloom > 0.0;
        let gtao = Gtao::new(&ctx.device, &ctx.shaders)?;
        let dust = if ctx.device.features().ray_query {
            Some(DustVolume::new(&ctx.device, &ctx.shaders)?)
        } else {
            None
        };
        let dust_on = !args.no_dust;
        let ao_on = !args.no_ao;
        // The Sun's disc softens the shadows between the rocks when asked (issues #54, #55).
        if args.soft_shadows {
            renderer.sun_angular_radius = forge_render::starfield::SUN_ANGULAR_RADIUS_1AU;
        }
        let tonemap = args.tonemap;
        let mut ballad = Self {
            args,
            renderer,
            starfield,
            atmosphere,
            taa,
            bloom,
            bloom_on,
            dust,
            dust_on,
            gtao,
            ao_on,
            taa_enabled,
            dlss,
            dlss_on,
            display,
            render_extent: ctx.extent(),
            meter,
            hasher,
            exposure,
            tonemap,
            step: 0.0,
            exposure_log,
            scene,
            path,
            path_t: 0.0,
            paused: false,
            camera,
            flags,
            wireframe: false,
            stats: Vec::new(),
            gpu_ms: Vec::new(),
            cpu_ms: Vec::new(),
            frame_ms: Vec::new(),
            last_frame: Instant::now(),
            title_updates: 0,
        };
        ballad.apply_upscaler(ctx.extent())?;
        Ok(ballad)
    }

    /// Draws at the size the anti-aliasing wants for an `output` of that many pixels: the
    /// window's with TAA, DLSS's input size with DLSS, with the jitter sequence to match. The
    /// device must be idle.
    fn apply_upscaler(&mut self, output: vk::Extent2D) -> Result<()> {
        let render = match &mut self.dlss {
            Some(dlss) if self.dlss_on => {
                dlss.configure(dlss.mode(), output)?;
                dlss.render_extent()?
            }
            _ => output,
        };
        if render != self.render_extent {
            self.renderer.resize(render)?;
            self.taa.resize(render)?;
            self.render_extent = render;
        }
        self.taa.jitter_phases = forge_render::taa::jitter_phases(render, output);
        self.taa.reset_history();
        Ok(())
    }

    /// TAA → DLAA → Quality → Balanced → Performance → Ultra Performance → TAA (the U key).
    fn cycle_upscaler(&mut self, ctx: &Context) {
        let Some(dlss) = &mut self.dlss else {
            tracing::warn!(
                "DLSS is not available (--features dlss, the Streamline SDK, an RTX GPU)"
            );
            return;
        };
        let next = if self.dlss_on {
            let index = DlssMode::ALL.iter().position(|&m| m == dlss.mode());
            index.and_then(|i| DlssMode::ALL.get(i + 1).copied())
        } else {
            Some(DlssMode::ALL[0])
        };
        ctx.device.wait_idle();
        let applied: Result<()> = match next {
            Some(mode) => {
                self.dlss_on = true;
                dlss.configure(mode, ctx.extent()).map_err(Into::into)
            }
            None => {
                self.dlss_on = false;
                Ok(())
            }
        };
        match applied.and_then(|()| self.apply_upscaler(ctx.extent())) {
            Ok(()) => tracing::info!(
                upscaler = self.upscaler_label(),
                render = ?self.render_extent,
                "anti-aliasing switched"
            ),
            Err(error) => tracing::error!(%error, "cannot switch the anti-aliasing"),
        }
    }

    /// The anti-aliasing on screen, for the overlay and the title.
    fn upscaler_label(&self) -> &'static str {
        match &self.dlss {
            Some(dlss) if self.dlss_on => dlss.mode().label(),
            _ => "TAA",
        }
    }

    fn cull_camera(&self, aspect: f32) -> CullCamera {
        CullCamera::new(
            self.camera.view(),
            self.camera.projection(aspect),
            self.camera.position,
            self.camera.near,
        )
    }
}

impl Demo for Ballad {
    fn resized(&mut self, ctx: &mut Context) -> Result<()> {
        self.apply_upscaler(ctx.extent())?;
        Ok(())
    }

    fn key_pressed(&mut self, ctx: &mut Context, code: KeyCode) {
        match code {
            KeyCode::KeyU => self.cycle_upscaler(ctx),
            KeyCode::KeyP => self.paused = !self.paused,
            KeyCode::KeyT => {
                self.taa_enabled = !self.taa_enabled;
                self.taa.reset_history();
            }
            KeyCode::KeyM => self.flags.toggle(CullFlags::MESHLET_COLORS),
            KeyCode::KeyO => self.flags.toggle(CullFlags::OCCLUSION),
            KeyCode::KeyC => self.flags.toggle(CullFlags::CONE),
            KeyCode::KeyX => self.flags.toggle(CullFlags::SHOW_CULLED),
            KeyCode::KeyL => self.flags.toggle(CullFlags::LOD),
            KeyCode::KeyK => self.flags.toggle(CullFlags::LOD_COLORS),
            KeyCode::KeyR => self.args.sw_raster = self.args.sw_raster.next(),
            KeyCode::KeyH => self.flags.toggle(CullFlags::SHOW_RASTER),
            KeyCode::BracketLeft => self.args.lod_error = (self.args.lod_error * 0.5).max(0.125),
            KeyCode::BracketRight => self.args.lod_error = (self.args.lod_error * 2.0).min(16.0),
            KeyCode::Tab => self.wireframe = !self.wireframe,
            KeyCode::KeyG => self.tonemap = self.tonemap.next(),
            KeyCode::KeyB => self.bloom_on = !self.bloom_on,
            KeyCode::KeyN => self.ao_on = !self.ao_on,
            KeyCode::KeyV => self.dust_on = !self.dust_on,
            KeyCode::KeyY => self.flags.toggle(CullFlags::TRANSLUCENCY),
            KeyCode::KeyZ => {
                self.renderer.sun_angular_radius = if self.renderer.sun_angular_radius > 0.0 {
                    0.0
                } else {
                    forge_render::starfield::SUN_ANGULAR_RADIUS_1AU
                };
            }
            KeyCode::KeyJ => self.flags.toggle(CullFlags::SHADOWS),
            KeyCode::Minus => self.exposure.compensation -= 0.5,
            KeyCode::Equal => self.exposure.compensation += 0.5,
            _ => {}
        }
    }

    fn update(&mut self, ctx: &mut Context, input: &Input, dt: f32) {
        if let Some(every) = self.args.cycle_upscaler
            && ctx.frames_rendered > 0
            && ctx.frames_rendered.is_multiple_of(every)
        {
            self.cycle_upscaler(ctx);
        }
        let now = Instant::now();
        self.frame_ms
            .push((now - self.last_frame).as_secs_f64() * 1e3);
        self.last_frame = now;
        let step = if self.args.fixed_step {
            1.0 / 120.0
        } else {
            dt
        };
        self.step = step;
        if self.paused {
            self.camera.update(input, dt);
            return;
        }
        self.path_t += step / self.args.duration;
        let position = self.path.sample(self.path_t);
        let ahead = self.path.sample(self.path_t + 0.004);
        let forward = match self.args.look {
            Some(look) => look.normalize_or(Vec3::NEG_Z),
            None => (ahead - position).normalize_or_zero(),
        };
        // Look along the tangent with a gentle roll into the turns.
        let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        self.camera.position = position;
        self.camera.yaw = -flat.x.atan2(-flat.z);
        self.camera.pitch = forward.y.asin().clamp(-1.2, 1.2);
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        let cpu_start = Instant::now();
        let frame_stats = self.renderer.begin_frame(frame.slot, &mut self.scene)?;
        let frame_hashes = self
            .hasher
            .as_ref()
            .map(|h| h.take(frame.slot))
            .unwrap_or_default();
        if let Some(stats) = frame_stats {
            self.stats.push(stats);
            if let Some(ms) = frame.slot.previous_gpu_ms {
                self.gpu_ms.push(ms);
            }
        }
        if let Some(last) = self.stats.last() {
            ctx.profile.counter(format!(
                "scene: {} asteroids, {} meshes, {:.0} M triangles, {:.1} M meshlets in the culling universe",
                self.scene.instance_count,
                self.scene.mesh_count,
                self.scene.total_triangles as f64 / 1e6,
                self.scene.instance_meshlets() as f64 / 1e6
            ));
            ctx.profile.counter(format!(
                "drawn through {}: {} instances{}, {:.0} k + {:.0} k meshlets, {:.2} M triangles, {:.0} k occluded{}",
                self.renderer.path().name(),
                last.instances_visible,
                last.hidden_note(),
                f64::from(last.meshlets_pass1) / 1e3,
                f64::from(last.meshlets_pass2) / 1e3,
                f64::from(last.triangles) / 1e6,
                f64::from(last.occluded) / 1e3,
                last.overflow_note()
            ));
            ctx.profile.counter(last.software_line(self.args.sw_raster));
            ctx.profile.counter(format!(
                "LOD {} at {:.2} px: mean level {:.2} of the drawn clusters; {} clusters in the DAG tables",
                if self.flags.has(CullFlags::LOD) { "on" } else { "off" },
                self.args.lod_error,
                f64::from(last.lod_level_sum) / f64::from((last.meshlets_pass1 + last.meshlets_pass2).max(1)),
                self.scene.meshlet_count
            ));
            ctx.profile.counter(format!(
                "{} (U) at {}x{}   occlusion {}   cone {}   {}",
                if self.dlss_on {
                    self.upscaler_label().to_owned()
                } else if self.taa_enabled {
                    "TAA on (T)".to_owned()
                } else {
                    "TAA off (T)".to_owned()
                },
                self.render_extent.width,
                self.render_extent.height,
                if self.flags.has(CullFlags::OCCLUSION) {
                    "on"
                } else {
                    "off"
                },
                if self.flags.has(CullFlags::CONE) {
                    "on"
                } else {
                    "off"
                },
                if self.paused {
                    "path paused (P)"
                } else {
                    "on the path (P to fly)"
                }
            ));
        }
        // Exposure for this frame from the histogram of the frame that last used this slot.
        let histogram = self.meter.take(frame.slot);
        self.exposure.update(histogram.as_ref(), self.step);
        let exposure = self.exposure.exposure();
        if let Some(log) = &mut self.exposure_log {
            writeln!(
                log,
                "{},{:.6},{:.4},{:.4}",
                ctx.frames_rendered, self.path_t, self.exposure.ev100, self.exposure.target_ev100
            )?;
        }
        ctx.profile.counter(format!(
            "exposure: EV100 {:.2} {} (target {:.2}, compensation {:+.1} EV with - / =); {} (G); sun {:.0} klux",
            self.exposure.ev100,
            if self.exposure.automatic { "auto" } else { "fixed" },
            self.exposure.target_ev100,
            self.exposure.compensation,
            self.tonemap.label(),
            self.args.sun_lux / 1000.0
        ));
        let cull = self.cull_camera(ctx.aspect());
        // The scene's size: the window's, or DLSS's input size.
        let extent = self.render_extent;
        if let Some(path) = std::env::var_os("FORGE_TRACE_FRAMES") {
            // Debugging aid: every CPU-side input of the frame, one line per frame, to diff two runs.
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let prev = self
                    .taa
                    .previous_view_proj()
                    .map(|m| m.to_cols_array())
                    .unwrap_or([0.0; 16]);
                let _ = writeln!(
                    file,
                    "{} t={:.9} pos={:?} yaw={} pitch={} taa={} flags={} prev={:?} gpu={:?} hashes={:x?}",
                    ctx.frames_rendered,
                    self.path_t,
                    self.camera.position.to_array(),
                    self.camera.yaw,
                    self.camera.pitch,
                    self.taa.frame_index(),
                    self.flags.0,
                    prev,
                    frame_stats,
                    frame_hashes
                );
            }
        }
        // Draw jittered into the HDR target; cull with the unjittered camera. The rocks go
        // first, then the sky fills the pixels they left (depth-tested), then the resolve.
        // DLSS needs the jitter whatever T says.
        self.taa.enabled = self.taa_enabled || self.dlss_on;
        // The soft shadows' noise repeats with the jitter (issue #54).
        self.renderer.noise_frame =
            (self.taa.frame_index() % u64::from(self.taa.jitter_phases)) as u32;
        let taa_frame = self.taa.begin(
            &mut frame.graph,
            self.camera.projection(ctx.aspect()),
            cull.view_proj,
            exposure,
        );
        let draw_view_proj = taa_frame.jittered_projection * self.camera.view();
        // `FORGE_HASH_IMAGES=1` (issue #71): the scene colour after each pass that writes it
        // (after the shading, the sky's pixels are still undefined), then the frame's other
        // images, in the frame trace.
        use forge_render::debug_hash::HashKind;
        let hasher = self.hasher.as_ref();
        if let Some(h) = hasher {
            h.begin(&mut frame.graph, frame.slot);
        }
        let targets = self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: draw_view_proj,
                cull,
                // The LOD error is meant in output pixels: drawn below the output (DLSS), the same
                // error is a smaller share of a render pixel, so the geometry stays as detailed
                // as on screen instead of coarsening with the render size.
                lod_threshold_px: self.args.lod_error * extent.height as f32
                    / ctx.extent().height.max(1) as f32,
                draw_jitter: taa_frame.jitter
                    / glam::Vec2::new(extent.width as f32, extent.height as f32),
                flags: self.flags,
                extent,
                wireframe: self.wireframe,
                exposure,
                sw_raster: self.args.sw_raster,
                instance_occlusion: self.args.instance_occlusion,
                sw_raster_area: self.args.sw_raster_area,
            },
        )?;
        // The fill light, occluded by what the depth shows around each pixel (issue #55).
        let occlusion = self.ao_on.then(|| {
            self.gtao.draw(
                &mut frame.graph,
                targets.depth,
                extent,
                GtaoParams {
                    projection: taa_frame.jittered_projection,
                    frame: self.taa.frame_index() % u64::from(self.taa.jitter_phases),
                    radius: self.args.ao_radius,
                },
            )
        });
        self.renderer.resolve(
            &mut frame.graph,
            frame.slot,
            targets,
            taa_frame.color,
            extent,
            None,
            AmbientLight {
                sky: None,
                occlusion,
                probes: None,
            },
        );
        if let Some(h) = hasher {
            h.add(&mut frame.graph, taa_frame.color, HashKind::Float4, extent);
        }
        let planet = self
            .atmosphere
            .as_mut()
            .map(|(atmosphere, view)| atmosphere.frame(&mut frame.graph, frame.slot, *view));
        self.starfield.draw(
            &mut frame.graph,
            taa_frame.color,
            targets.depth,
            extent,
            draw_view_proj,
            self.renderer.sun_dir,
            exposure,
            planet,
        );
        if let Some(h) = hasher {
            h.add(&mut frame.graph, taa_frame.color, HashKind::Float4, extent);
        }
        // The belt's dust, lit by the sun between the rocks (issue #58).
        if let Some(dust) = self.dust.as_ref().filter(|_| self.dust_on) {
            let sun_luminance = self.renderer.sun_illuminance * exposure;
            dust.draw(
                &mut frame.graph,
                frame.slot,
                DustParams {
                    view_proj: draw_view_proj,
                    camera: self.camera.position,
                    sun_dir: self.renderer.sun_dir,
                    sun_color: self.renderer.sun_color,
                    sun_luminance,
                    extinction: self.args.dust,
                    far: 700.0,
                    anisotropy: 0.7,
                    fill: Vec3::new(0.10, 0.12, 0.18) * 0.02 * sun_luminance,
                    tlas: self.scene.rays().map_or(0, |r| r.tlas_address()),
                    frame: (self.taa.frame_index() % u64::from(self.taa.jitter_phases)) as u32,
                },
                targets.depth,
                taa_frame.color,
                extent,
            );
        }
        if let Some(h) = hasher {
            h.add(&mut frame.graph, taa_frame.color, HashKind::Float4, extent);
        }
        // Meter the finished HDR scene (the next frames' exposure), then resolve it: TAA into
        // its history and, through the tone curve, the swapchain; or DLSS into an HDR image at
        // the window's size that the display pass takes through the curve.
        self.meter.measure(
            &mut frame.graph,
            frame.slot,
            taa_frame.color,
            extent,
            exposure,
        );
        let motion = self
            .taa
            .motion_vectors(&mut frame.graph, &taa_frame, targets.depth);
        let (history, bloom_image) = match self.dlss.as_mut() {
            Some(dlss) if self.dlss_on => {
                let upscaled = dlss.upscale(
                    &mut frame.graph,
                    ctx.frames_rendered,
                    &taa_frame,
                    UpscaleCamera {
                        projection: self.camera.projection(ctx.aspect()),
                        near: self.camera.near,
                        vertical_fov: self.camera.fov_y,
                    },
                    targets.depth,
                    motion,
                    exposure,
                )?;
                self.display.draw(
                    &mut frame.graph,
                    upscaled,
                    frame.target,
                    ctx.extent(),
                    self.tonemap,
                );
                (None, None)
            }
            _ => {
                let bloom = self.bloom_on.then(|| {
                    self.bloom
                        .draw(&mut frame.graph, taa_frame.color, taa_frame.extent)
                });
                let history = self.taa.resolve(
                    &mut frame.graph,
                    &taa_frame,
                    targets.depth,
                    motion,
                    frame.target,
                    self.tonemap,
                    bloom,
                );
                (Some(history), bloom)
            }
        };
        if let Some(h) = hasher {
            let half = vk::Extent2D {
                width: (extent.width >> 1).max(1),
                height: (extent.height >> 1).max(1),
            };
            let images = [
                Some((targets.visibility, HashKind::Uint, extent)),
                Some((targets.depth, HashKind::Depth, extent)),
                Some((motion, HashKind::Float4, extent)),
                occlusion.map(|ao| (ao, HashKind::Float4, extent)),
                bloom_image.map(|b| (b, HashKind::Float4, half)),
                history.map(|h| (h, HashKind::Float4, extent)),
            ];
            for (image, kind, size) in images.into_iter().flatten() {
                h.add(&mut frame.graph, image, kind, size);
            }
            h.finish(&mut frame.graph);
        }
        self.cpu_ms.push(cpu_start.elapsed().as_secs_f64() * 1e3);
        Ok(())
    }

    fn title(&mut self, _ctx: &Context) -> Option<String> {
        if self.stats.is_empty() {
            return None;
        }
        let n = self.stats.len() as f64;
        let mean =
            |f: fn(&FrameStats) -> u32| self.stats.iter().map(|s| f64::from(f(s))).sum::<f64>() / n;
        let gpu = self.gpu_ms.iter().sum::<f64>() / self.gpu_ms.len().max(1) as f64;
        let cpu = self.cpu_ms.iter().sum::<f64>() / self.cpu_ms.len().max(1) as f64;
        let mut frames = std::mem::take(&mut self.frame_ms);
        frames.sort_by(f64::total_cmp);
        let p = |q: f64| {
            frames
                .get(((frames.len().max(1) - 1) as f64 * q) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        let title = format!(
            "forge asteroids | {} asteroids, {} meshes, {:.1} M meshlets, {:.0} M tris | {}: drawn {:.0} k + {:.0} k meshlets ({:.0} k in software, {:.2} M dense triangles; {:.0} k work items, {:.0} k roots), {:.2} M tris | GPU {:.2} ms  CPU {:.2} ms  frame p50 {:.2} p99 {:.2} ms | EV100 {:.1} {} | {}{}{}{}{}",
            self.scene.instance_count,
            self.scene.mesh_count,
            self.scene.instance_meshlets() as f64 / 1e6,
            self.scene.total_triangles as f64 / 1e6,
            self.renderer.path().name(),
            mean(|s| s.meshlets_pass1) / 1e3,
            mean(|s| s.meshlets_pass2) / 1e3,
            mean(|s| s.sw_clusters) / 1e3,
            mean(|s| s.dense_triangles) / 1e6,
            mean(|s| s.work_items) / 1e3,
            mean(|s| s.root_entries) / 1e3,
            mean(|s| s.triangles) / 1e6,
            gpu,
            cpu,
            p(0.5),
            p(0.99),
            self.exposure.ev100,
            self.tonemap.name(),
            if self.paused { "[P paused] " } else { "" },
            if self.dlss_on {
                format!("[U {}] ", self.upscaler_label())
            } else if self.taa_enabled {
                "[T taa] ".to_owned()
            } else {
                String::new()
            },
            if self.flags.has(CullFlags::OCCLUSION) {
                "[O occlusion] "
            } else {
                ""
            },
            if self.flags.has(CullFlags::MESHLET_COLORS) {
                "[M colours] "
            } else {
                ""
            },
            if self.wireframe { "[Tab wire]" } else { "" },
        );
        self.stats.clear();
        self.gpu_ms.clear();
        self.cpu_ms.clear();
        self.title_updates += 1;
        if self.title_updates.is_multiple_of(4) {
            tracing::info!("{title}");
        }
        Some(title)
    }
}

/// Ice shapes per size class (issue #63).
const ICE_SHAPES: usize = 2;

/// Which of the ballad's three ices an ice asteroid is made of (issue #61), by the third byte
/// of its instance hash: 35 % clear, 40 % bubbly, 25 % white.
fn ice_kind(id: u32) -> usize {
    match forge_render::material::instance_hash(id)[2] {
        0..=88 => 0,
        89..=190 => 1,
        _ => 2,
    }
}

/// Mesh recipes, one per size class: (segments per face, radius, roughness).
const RECIPES: [(u32, f32, f32); 7] = [
    (48, 1.0, 0.45),
    (64, 1.8, 0.40),
    (72, 2.6, 0.35),
    (96, 4.0, 0.30),
    (128, 7.0, 0.28),
    (160, 14.0, 0.25),
    (192, 30.0, 0.22),
];

/// Rock shapes per size class (`--variants`, issue #23).
fn rock_variants(args: &Args) -> usize {
    args.variants.max(1) as usize
}

/// Ice shapes per size class: none when the ice takes the rock's (issue #63).
fn ice_shapes(args: &Args) -> usize {
    if args.rock_shaped_ice || args.round_rocks {
        0
    } else {
        ICE_SHAPES
    }
}

/// The field's meshes and how long they took: the start-up's heavy CPU work, which runs
/// behind the loading screen (issue #25).
struct FieldMeshes {
    meshes: Vec<MeshletMesh>,
    build_ms: u128,
}

/// Builds the field's meshes on the job system (CPU only: no GPU context).
fn build_meshes(args: &Args) -> FieldMeshes {
    let start = Instant::now();
    let pool = TaskPool::client();
    let recipes = RECIPES;
    // Every size class in `args.variants` rock shapes (issue #23): each cut by its own planes,
    // and all but the first stretched along two axes, as real asteroids are rarely round. Then
    // the ice's own shapes, ICE_SHAPES per class (issue #63): a smoother body cut by more
    // planes, so the ice reads as blocks. Built in parallel; class `c`'s rock variant `v` is
    // `meshes[c * variants + v]`, its ice shape `v` follows all the rock.
    let variants = rock_variants(args);
    let round = args.round_rocks;
    let ice_shapes = ice_shapes(args);
    let jobs: Vec<(usize, usize, bool)> = (0..recipes.len())
        .flat_map(|i| (0..variants).map(move |v| (i, v, false)))
        .chain((0..recipes.len()).flat_map(|i| (0..ice_shapes).map(move |v| (i, v, true))))
        .collect();
    let mut meshes: Vec<Option<MeshletMesh>> = jobs.iter().map(|_| None).collect();
    let no_crust = args.no_crust;
    let lod_normals = args.lod_normals;
    pool.scope(|s| {
        for (slot, &(i, v, ice)) in meshes.iter_mut().zip(&jobs) {
            let (segments, radius, roughness) = recipes[i];
            s.spawn(move |_| {
                // Angular chunks with fractured facets (issue #60), or the Phase 0 round rocks.
                let seed = Seed::new(if ice { 900 } else { 700 } + i as u64 + 100 * v as u64);
                let mut mesh = if round {
                    procedural::asteroid(seed, segments, radius, roughness)
                } else if ice {
                    procedural::chunk(seed, segments, radius, roughness * 0.3, 20 + (i + v) as u32)
                } else {
                    procedural::chunk(seed, segments, radius, roughness, 8 + (i + v) as u32)
                };
                if v > 0 {
                    // Axis ratios of 1 : 0.6–0.95 : 0.45–that, the longest axis kept.
                    let mut rng = seed.derive_str("stretch").rng();
                    let b = 0.6 + 0.35 * rng.next_f32();
                    let c = 0.45 + (b - 0.45) * rng.next_f32();
                    for p in &mut mesh.positions {
                        *p = [p[0], p[1] * b, p[2] * c];
                    }
                    mesh.recompute_normals();
                }
                if no_crust {
                    mesh.sections.clear();
                }
                // In proportion to the radius, the normal change a level may make depends only on
                // the rock's size on screen: its creases and relief stay until they are a pixel.
                let options = CookOptions {
                    normal_weight: lod_normals * radius,
                };
                *slot = Some(MeshletMesh::build_with(&mesh, options));
            });
        }
    });
    FieldMeshes {
        meshes: meshes
            .into_iter()
            .map(|mesh| mesh.expect("mesh built"))
            .collect(),
        build_ms: start.elapsed().as_millis(),
    }
}

/// The field: seven size classes of asteroid meshes in several shapes, thousands of instances
/// clustered along a curved belt, and a camera path weaving through it, from the meshes
/// [`build_meshes`] made.
fn build_field(ctx: &Context, args: &Args, field: FieldMeshes) -> Result<(MeshletScene, Path)> {
    let start = Instant::now();
    let recipes = RECIPES;
    let variants = rock_variants(args);
    let ice_shapes = ice_shapes(args);
    let meshes = field.meshes;
    let mut builder = MeshletSceneBuilder::new();
    // Rock and ice: a fifth of the asteroids are ice (the rule since Phase 0). The rock takes the
    // procedural rock texture and its relief (issue #46); the ice stays smooth.
    let mut materials = MaterialTable::new();
    let mut textures = TextureSet::new(&ctx.device);
    // Every row is followed by the one its chunks' fracture faces take (section 1, issue #62).
    // A chunk's old surface is crust that space weathering has darkened and reddened; its
    // fracture faces show the rock beneath. The ice is the same ice on both.
    let mut add_with_faces = |row: Material, faces: Material| {
        let id = materials.add(row);
        materials.add(faces);
        id
    };
    let rock_row = if args.no_textures {
        stock::rock()
    } else {
        let [albedo, normal] = forge_render::textures::rock(11, 512);
        let (albedo, normal) = (textures.add(&albedo)?, textures.add(&normal)?);
        let mut rock = stock::rock();
        // The texture averages about 0.37: the Phase 0 colours over that.
        rock.render.color_a = [1.13, 1.08, 1.03];
        rock.render.color_b = [1.22, 0.89, 0.68];
        rock.render.albedo_texture = Some(albedo);
        rock.render.normal_texture = Some(normal);
        rock.render.texture_scale = 4.0;
        // Its repeats hidden on the big fracture faces (issue #66).
        rock.render.hex_tiling = !args.no_hex_tiling;
        rock
    };
    let weathered = |[r, g, b]: [f32; 3]| [r * 0.72, g * 0.62, b * 0.55];
    let mut crust = rock_row.clone();
    if !args.no_crust {
        crust.name = "weathered rock".to_owned();
        crust.render.color_a = weathered(rock_row.render.color_a);
        crust.render.color_b = weathered(rock_row.render.color_b);
    }
    let rock = add_with_faces(crust, rock_row);
    // Ice of three densities (issue #61): clear blocks glow deep blue through metres of ice,
    // bubbly ones turn white and glow only at their thin edges.
    let ice_rows = if args.clear_ice {
        [add_with_faces(stock::ice(), stock::ice()); 3]
    } else {
        [("clear ice", 3e-5), ("ice", 6e-4), ("white ice", 3e-3)].map(|(name, bubbles)| {
            add_with_faces(
                stock::bubbly_ice(name, bubbles),
                stock::bubbly_ice(name, bubbles),
            )
        })
    };
    builder.set_materials(&materials, Some(textures));
    // The field stays still until Phase 3: its structures are built once (issue #45).
    builder.set_ray_traced(!args.no_shadows);
    let mesh_ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    for (i, mesh) in meshes.iter().enumerate() {
        tracing::info!(mesh = i, levels = ?mesh.clusters_per_level, dag_triangles = mesh.dag_triangle_count, "cluster DAG");
    }
    let mesh_ms = field.build_ms;

    // The belt: an S-shaped centre line; asteroids scattered around it with density peaks.
    let mut rng: SplitMix64 = Seed::new(4242).rng();
    let length = args.length;
    let centre = |t: f32| {
        Vec3::new(
            (t - 0.5) * length,
            (t * std::f32::consts::TAU).sin() * 40.0,
            (t * 3.9).sin() * 120.0,
        )
    };
    // The path follows the belt centre with an offset that swings in and out of the clumps,
    // and closes far outside the belt so the camera turns around in open space.
    let mut points: Vec<Vec3> = (0..24)
        .map(|i| {
            let t = i as f32 / 24.0;
            let swing = Vec3::new(
                (t * 12.0).sin() * 45.0,
                (t * 9.0).cos() * 25.0,
                (t * 7.0).sin() * 45.0,
            );
            centre(t) + swing
        })
        .collect();
    let end = *points.last().expect("points");
    let first = points[0];
    points.push(end + Vec3::new(200.0, 120.0, 300.0));
    points.push(first + Vec3::new(-200.0, 120.0, 300.0));
    let path = Path { points };
    // A dense polyline of the path to keep a flight corridor clear of rock.
    let corridor: Vec<Vec3> = (0..2048).map(|i| path.sample(i as f32 / 2048.0)).collect();
    let corridor_clearance = |p: Vec3, radius: f32| {
        corridor
            .iter()
            .all(|c| c.distance_squared(p) > (radius + 14.0) * (radius + 14.0))
    };
    // No two rocks may overlap: a coarse grid of what was placed so far.
    const CELL: f32 = 80.0;
    let mut occupancy: std::collections::HashMap<(i32, i32, i32), Vec<(Vec3, f32)>> =
        std::collections::HashMap::new();
    let cell_of = |p: Vec3| {
        (
            (p.x / CELL).floor() as i32,
            (p.y / CELL).floor() as i32,
            (p.z / CELL).floor() as i32,
        )
    };
    let mut placed = 0;
    let mut attempts = 0;
    while placed < args.count && attempts < args.count * 40 {
        attempts += 1;
        let t = rng.next_f32();
        let id = builder.instance_count() as u32;
        let ice = stock::is_ice(id);
        // With `--ice-belt`, the ice keeps to a belt of its own along the same orbit (issue #22).
        let c = if ice {
            centre(t) + Vec3::Z * args.ice_belt
        } else {
            centre(t)
        };
        // Clumps every ~80 m along the belt, thinner in between.
        let clump = (t * length / 80.0 * std::f32::consts::TAU).cos() * 0.5 + 0.5;
        if rng.next_f32() > 0.25 + 0.75 * clump {
            continue;
        }
        // Uniform in a flattened disc around the belt line: wide, thin, denser in clumps.
        let angle = rng.range_f32(0.0, std::f32::consts::TAU);
        let radius = rng.next_f32().sqrt() * (140.0 + 220.0 * clump);
        let position = c + Vec3::new(
            angle.cos() * radius * 0.6,
            rng.range_f32(-1.0, 1.0) * 70.0,
            angle.sin() * radius,
        );
        // Big ones are rare: pick the mesh with a strongly skewed distribution.
        let pick = rng.next_f32().powf(2.5) * recipes.len() as f32;
        let mesh = pick.floor().min(recipes.len() as f32 - 1.0) as usize;
        let scale = rng.range_f32(0.6, 1.5);
        let world_radius = recipes[mesh].1 * scale * 1.5;
        if !corridor_clearance(position, world_radius) {
            continue;
        }
        let (cx, cy, cz) = cell_of(position);
        let reach = (world_radius / CELL).ceil() as i32 + 1;
        let mut overlaps = false;
        'cells: for dz in -reach..=reach {
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    if let Some(list) = occupancy.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &(other, other_radius) in list {
                            let gap = world_radius + other_radius + 3.0;
                            if other.distance_squared(position) < gap * gap {
                                overlaps = true;
                                break 'cells;
                            }
                        }
                    }
                }
            }
        }
        if overlaps {
            continue;
        }
        occupancy
            .entry((cx, cy, cz))
            .or_default()
            .push((position, world_radius));
        let rotation = Quat::from_euler(
            glam::EulerRot::XYZ,
            rng.range_f32(0.0, std::f32::consts::TAU),
            rng.range_f32(0.0, std::f32::consts::TAU),
            rng.range_f32(0.0, std::f32::consts::TAU),
        );
        // The shape within its class, by a hash of the instance: the placement draws the same
        // numbers as with one shape.
        let shape = hash_cell3(0x5EED_0023, id as i32, 0, 0);
        let shape = if ice && ice_shapes > 0 {
            recipes.len() * variants + mesh * ice_shapes + (shape % ice_shapes as u64) as usize
        } else {
            mesh * variants + (shape % variants as u64) as usize
        };
        builder.add_instance_with_material(
            mesh_ids[shape],
            Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, position),
            if ice { ice_rows[ice_kind(id)] } else { rock },
        );
        placed += 1;
    }
    let mut scene = builder.build(&ctx.device)?;
    scene.build_tlas(&ctx.device, &ctx.shaders)?;
    if let Some(rays) = scene.rays() {
        tracing::info!(
            blas_triangles = rays.triangles,
            mib = rays.bytes() >> 20,
            blas_ms = %format_args!("{:.0}", rays.blas_ms),
            tlas_ms = %format_args!("{:.0}", rays.tlas_ms),
            "acceleration structures"
        );
    }
    tracing::info!(
        meshes = mesh_ids.len(),
        mesh_build_ms = mesh_ms,
        placement_attempts = attempts,
        instances = scene.instance_count,
        meshlets = scene.instance_meshlets(),
        total_triangles = scene.total_triangles,
        total_ms = mesh_ms + start.elapsed().as_millis(),
        "asteroid field ready"
    );
    Ok((scene, path))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = AppConfig {
        title: "forge asteroids".into(),
        vsync: args.vsync,
        validate: args.validate,
        frame_limit: args.frames,
        capture: args.capture.clone().map(|p| (p, args.capture_frame)),
        capture_every: args.capture_every,
        overlay: if args.overlay {
            Some(true)
        } else if args.no_overlay {
            Some(false)
        } else {
            None
        },
        // Built with `--features dlss`: the Vulkan API comes through Streamline so U can switch
        // to DLSS at run time.
        streamline: cfg!(feature = "dlss"),
        force_fallback: args.force_fallback,
        width: args.width,
        height: args.height,
        ..AppConfig::default()
    };
    // The meshes and their cluster DAGs (about 2.4 s) build behind the loading screen (issue #25);
    // the uploads follow on the main thread.
    forge_app::run_loading(config, move || {
        let field = build_meshes(&args);
        let finish: Finish<Ballad> = Box::new(move |ctx| Ballad::new(ctx, args, field));
        Ok(finish)
    })
}

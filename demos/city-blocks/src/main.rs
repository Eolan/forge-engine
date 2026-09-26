//! `city-blocks` — the Phase 1 closing demo (issue #13), built in steps. The twenty
//! procedural props of the city set (0.5 to 3 M triangles each, issue #34) and a 4 km
//! terrain (8 M triangles) are cooked into cluster DAGs once and cached on disk
//! (`mesh-cache/`); a compute pass places a million instances of the props over the terrain
//! (issue #35): a street grid of buildings, lamp posts and plazas, and rocks over the hills
//! around it. Their cluster pages stream from the cache files through a GPU pool as the LOD
//! cut asks for them (issue #36); `--fly` flies a loop at 300 m/s through TAA (issue #13).
//! `--gallery` shows the twenty props side by side instead.
//!
//! Controls: WASD/QE move, Shift fast, right mouse look, L cluster LOD, K LOD colours, M
//! cluster colours, O occlusion, R software rasteriser, H show what it drew, [ / ] LOD
//! threshold, T TAA, B bloom, J shadows, I sky light, N ambient occlusion, V its view, F sky
//! reflections, Y mirror rays in the glass, Z soft or hard shadows, Tab wireframe, G tone curve,
//! Esc quit.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_app::{AppConfig, Context, Demo, Finish, FlyCamera, FrameInfo, Input};
use forge_core::material::{
    Material, MaterialId, MaterialTable, RenderLayer, ShadingClass, TextureId,
};
use forge_geom::MeshletMesh;
use forge_geom::cache::cook_cached;
use forge_geom::city::{Heightfield, PropKind, PropSpec, Terrain, city_props};
use forge_procgen::{ErosionParams, Field2, IslandParams};
use forge_render::material::TextureSet;
use forge_render::meshlet::{DrawParams, MeshId};
use forge_render::placement::{self, CityLayout, CityMeshes, Ground};
use forge_render::textures::{self, TextureData};
use forge_render::{
    AmbientLight, Atmosphere, AtmosphereParams, AutoExposure, Bloom, CullCamera, CullFlags,
    FrameStats, GroundSky, Gtao, GtaoParams, LuminanceMeter, MeshletRenderer, MeshletScene,
    MeshletSceneBuilder, ProbeParams, Probes, Residency, SkyParams, StreamingConfig,
    StreamingStats, SwRaster, Taa, Tonemap, exposure_from_ev100, sh_irradiance,
};
use forge_task::TaskPool;
use glam::{Mat4, Vec3};
use winit::keyboard::KeyCode;

#[derive(Parser, Debug, Clone)]
#[command(about = "City blocks: the prop gallery")]
struct Args {
    /// Vertical sync.
    #[arg(long)]
    vsync: bool,
    /// Vulkan validation layer.
    #[arg(long)]
    validate: bool,
    /// Exit after this many frames.
    #[arg(long)]
    frames: Option<u64>,
    /// Write a PNG of frame `--capture-frame` to this path.
    #[arg(long)]
    capture: Option<PathBuf>,
    /// Which frame to capture.
    #[arg(long, default_value_t = 60)]
    capture_frame: u64,
    /// Scripted camera: circle the gallery (for headless comparisons).
    #[arg(long)]
    orbit: bool,
    /// Projected LOD error a drawn cluster may have, in pixels.
    #[arg(long, default_value_t = 1.0)]
    lod_error: f32,
    /// Draw the full-detail clusters only (L toggles it).
    #[arg(long)]
    no_lod: bool,
    /// Start with occlusion culling off (O toggles it).
    #[arg(long)]
    no_occlusion: bool,
    /// The software rasteriser: auto, on or off (R cycles them).
    #[arg(long, default_value = "auto")]
    sw_raster: SwRaster,
    /// Instance occlusion (issue #38): auto (while most instances in the frustum are hidden),
    /// on or off. Every mode must give the same image (A/B harness).
    #[arg(long, default_value = "auto")]
    instance_occlusion: forge_render::InstanceOcclusion,
    /// Cull the instances one by one instead of by cells of 64 first (issue #38; cells only
    /// in scenes of 65 536 instances or more). Both ways must give the same image.
    #[arg(long)]
    no_instance_cells: bool,
    /// The culling-error view: what culling rejected drawn in red (the A/B harness; with
    /// instance occlusion and cells, issue #38).
    #[arg(long)]
    show_culled: bool,
    /// Dense clusters: fewer pixels of bounding rectangle than this per triangle.
    #[arg(long, default_value_t = forge_render::meshlet::SW_RASTER_DEFAULT_AREA)]
    sw_raster_area: f32,
    /// Fixed exposure value at ISO 100 (15: sunny 16).
    #[arg(long, default_value_t = 15.0)]
    ev100: f32,
    /// Tone curve: agx, aces or neutral (G cycles them).
    #[arg(long, default_value = "agx")]
    tonemap: Tonemap,
    /// Force the profiling overlay on (also in scripted runs). F1 toggles it.
    #[arg(long)]
    overlay: bool,
    /// Draw through the indirect-count fallback: the device is created without mesh shaders.
    #[arg(long)]
    force_fallback: bool,
    /// Cook every prop again, ignoring (and replacing) the cache.
    #[arg(long)]
    recook: bool,
    /// Start framed on this prop (its name in the log, e.g. `fountain`; the gallery only).
    #[arg(long)]
    focus: Option<String>,
    /// Start the camera at `x,y,z,yaw,pitch`: metres, then degrees (yaw 0 looks north, along
    /// −z; 90 west; pitch up is positive), e.g. `--view=-8,1.7,1135,-50,10` in a street.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    view: Option<Vec<f32>>,
    /// Move the scene this far from the world's origin along every axis, metres (issue #93): the
    /// terrain, the instances, the camera and its paths go together (`--view` stays in the
    /// scene's metres), so a renderer without a precision limit would draw the same image. The
    /// renderer keeps positions in f32, whose spacing grows with the distance (the log prints
    /// it): the image drifts, the farther the more.
    #[arg(long, default_value_t = 0.0)]
    origin: f32,
    /// Draw the island of `forge-procgen` (`docs/demos/island.md`) instead of the city: a
    /// 16 km island generated from this seed, its heightfield cooked into a cluster DAG like the
    /// city's ground and cached. The first start generates it (a minute or two of erosion at
    /// 8 m), the next ones load it.
    #[arg(long)]
    island: Option<u64>,
    /// Metres between the island's samples (8: 2049², 8.4 M triangles; 4: the 4097² target).
    #[arg(long, default_value_t = 8.0)]
    island_spacing: f64,
    /// Erosion steps of the island.
    #[arg(long, default_value_t = 150)]
    island_steps: u32,
    /// The island's prevailing wind, by the compass point it blows from (n, ne, e, se, s,
    /// sw, w, nw): orographic rain, the windward slopes wetter. Without it the rain is flat.
    #[arg(long)]
    island_wind: Option<String>,
    /// How far the island's orographic rain departs from flat (0 flat, 1 the model).
    #[arg(long, default_value_t = 1.0)]
    island_rain_contrast: f64,
    /// Show the twenty props side by side instead of the city.
    #[arg(long)]
    gallery: bool,
    /// Instances placed over the terrain: 1 000 000 by default over the city (the city takes
    /// about 12 k, the hills the rest), 300 000 rocks on the island's land.
    #[arg(long)]
    instances: Option<u32>,
    /// Stream the city's cluster pages through a pool of this many MiB (0: every page
    /// resident, read once at start).
    #[arg(long, default_value_t = 512)]
    stream_pool: u32,
    /// Most MiB of cluster pages uploaded per frame while streaming.
    #[arg(long, default_value_t = 8)]
    stream_upload: u32,
    /// Fly a loop at 300 m/s, 140 m up, over the city's edge and the hills (1.4 km from the
    /// centre, 29 s a lap, in real time).
    #[arg(long)]
    fly: bool,
    /// Advance the flight by 1/60 s a frame instead of the frame's time (reproducible
    /// captures; with `--vsync` at 60 Hz, still 300 m/s).
    #[arg(long)]
    fixed_step: bool,
    /// The sun's elevation over the horizon, degrees (63.4 over the city, the renderer's default
    /// sun; 30 over the island, where a high sun flattens the relief).
    #[arg(long)]
    sun_elevation: Option<f32>,
    /// Draw without the sun's ray-traced shadows (J toggles them; devices without ray queries
    /// have none).
    #[arg(long)]
    no_shadows: bool,
    /// Light the shaded sides with space's constant fill instead of the sky's irradiance (I
    /// toggles it).
    #[arg(long)]
    no_sky_light: bool,
    /// Leave the sky's light unoccluded: no ambient occlusion (N toggles it).
    #[arg(long)]
    no_ao: bool,
    /// How far an occluder reaches for the ambient occlusion, metres.
    #[arg(long, default_value_t = 1.5)]
    ao_radius: f32,
    /// Light the shaded sides with the open sky's irradiance, without the probes' light: the
    /// sky the streets' buildings leave and the light bouncing off the city (P toggles them;
    /// devices without ray queries have none).
    #[arg(long)]
    no_probes: bool,
    /// Rays per probe and frame (64 to 256).
    #[arg(long, default_value_t = ProbeParams::default().rays)]
    probe_rays: u32,
    /// Probe cascades, 4 m apart for the finest and twice as far each after (1 to 6).
    #[arg(long, default_value_t = ProbeParams::default().cascades)]
    probe_cascades: u32,
    /// Show the diffuse light alone, on white surfaces, instead of the shading: the probes'
    /// (the open sky's with `--no-probes`); U toggles it.
    #[arg(long)]
    show_gi: bool,
    /// Show the ambient occlusion in grey instead of the shading (V toggles it).
    #[arg(long)]
    show_ao: bool,
    /// Draw without the sky's reflection in glass and at grazing angles (F toggles it).
    #[arg(long)]
    no_reflections: bool,
    /// Reflect only the sky in the glass, without the mirror rays against the city (Y toggles
    /// them; devices without ray queries have none).
    #[arg(long)]
    no_ray_reflections: bool,
    /// Hard sun shadows: one ray to the sun's centre instead of its disc (Z toggles them).
    #[arg(long)]
    hard_shadows: bool,
    /// A day over the city: the sun rises in the east, crosses the south and sets in the west in
    /// this many seconds, then again, and the exposure follows it (issue #57).
    #[arg(long)]
    day: Option<f32>,
    /// Bloom strength, the share of the shown image that is bloom (0 for none; B toggles it).
    #[arg(long, default_value_t = 0.04)]
    bloom: f32,
    /// Draw without TAA (no jitter, no history): the raw frame, aliased.
    #[arg(long)]
    no_taa: bool,
    /// Window width in pixels (the render size).
    #[arg(long, default_value_t = 1600)]
    width: u32,
    /// Window height in pixels.
    #[arg(long, default_value_t = 900)]
    height: u32,
}

/// Where a prop stands in the gallery: its name, the centre and radius of its bounds.
struct Placed {
    name: String,
    center: Vec3,
    radius: f32,
}

struct Gallery {
    args: Args,
    renderer: MeshletRenderer,
    /// Anti-aliasing: jittered frames into a history, resolved through the tone curve.
    taa: Taa,
    /// Bloom before the tone curve (issue #44), on while `bloom_on`.
    bloom: Bloom,
    bloom_on: bool,
    /// The Earth's atmosphere the city stands in, and the sky seen from the ground (issue #43).
    atmosphere: Atmosphere,
    sky: GroundSky,
    /// The shaded sides lit by the sky's irradiance (issue #47); else the old constant fill.
    sky_light: bool,
    /// Ambient occlusion of the sky's light (issue #48), on while `ao_on`.
    gtao: Gtao,
    ao_on: bool,
    /// Diffuse light from probes (issue #53), on while `probes_on`; `probes_live` while they
    /// were updated every frame (else they start over).
    probes: Option<Probes>,
    probes_on: bool,
    probes_live: bool,
    /// `--day`: seconds into the day, the metered scene and the automatic exposure (issue #57).
    day_time: f32,
    meter: LuminanceMeter,
    auto_exposure: AutoExposure,
    /// Seconds the last update advanced.
    step: f32,
    tonemap: Tonemap,
    scene: MeshletScene,
    camera: FlyCamera,
    flags: CullFlags,
    wireframe: bool,
    frame: u64,
    stats: Vec<FrameStats>,
    gpu_ms: Vec<f64>,
    title_updates: u32,
    /// Seconds flown (`--fly`).
    fly_time: f32,
    /// The streamer's frames since the last title.
    streaming: Vec<StreamingStats>,
    /// Frame times (ms) since the last title, and over the whole run (the exit log).
    frame_ms: Vec<f64>,
    run_frame_ms: Vec<f64>,
    last_frame: Instant,
    /// When the first frame was rendered, until the streamer first settles (nothing wanted
    /// or being read).
    started: Option<Instant>,
    settled: bool,
}

/// Metres between the centres of neighbouring props in the gallery.
const SPACING: f32 = 60.0;
/// Props per row of the gallery.
const COLUMNS: u32 = 5;

impl Gallery {
    fn new(ctx: &mut Context, args: Args, cooked: Cooked) -> Result<Self> {
        let mut renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, ctx.extent())?;
        let mut taa = Taa::new(
            &ctx.device,
            &ctx.shaders,
            ctx.extent(),
            ctx.swapchain.format(),
        )?;
        taa.enabled = !args.no_taa;
        taa.bloom_strength = args.bloom;
        let bloom = Bloom::new(&ctx.device, &ctx.shaders)?;
        let bloom_on = args.bloom > 0.0;
        let sky_light = !args.no_sky_light;
        let gtao = Gtao::new(&ctx.device, &ctx.shaders)?;
        let meter = LuminanceMeter::new(&ctx.device, &ctx.shaders)?;
        let auto_exposure = AutoExposure::new(args.ev100);
        let ao_on = !args.no_ao;
        // The sun at `--sun-elevation`, from the default sun's azimuth, through the air.
        let atmosphere_params = AtmosphereParams::earth();
        let default_elevation = if args.island.is_some() { 30.0 } else { 63.4 };
        let elevation = args.sun_elevation.unwrap_or(default_elevation).to_radians();
        renderer.sun_dir = Vec3::new(
            0.8 * elevation.cos(),
            elevation.sin(),
            0.6 * elevation.cos(),
        );
        renderer.sun_color = Vec3::from(atmosphere_params.transmittance(
            Vec3::new(0.0, atmosphere_params.bottom_radius + 0.05, 0.0),
            renderer.sun_dir,
            64,
        ));
        // The sun's disc softens the shadows (issue #54).
        if !args.hard_shadows {
            renderer.sun_angular_radius = forge_render::starfield::SUN_ANGULAR_RADIUS_1AU;
        }
        let atmosphere = Atmosphere::new(&ctx.device, &ctx.shaders, atmosphere_params)?;
        let sky = GroundSky::new(&ctx.device, &ctx.shaders)?;
        let (scene, placed) = if args.gallery {
            build_gallery(ctx, &args, cooked)?
        } else if args.island.is_some() {
            (build_island(ctx, &args, cooked)?, Vec::new())
        } else {
            (build_city(ctx, &args, cooked)?, Vec::new())
        };
        let mut flags = CullFlags(CullFlags::CONE | CullFlags::FRUSTUM);
        if !args.no_lod {
            flags.0 |= CullFlags::LOD;
        } else {
            renderer.reserve_visible(scene.finest_clusters);
        }
        if !args.no_occlusion {
            flags.0 |= CullFlags::OCCLUSION;
        }
        if !args.no_shadows {
            flags.0 |= CullFlags::SHADOWS;
        }
        if args.show_ao {
            flags.0 |= CullFlags::SHOW_AO;
        }
        if args.show_gi {
            flags.0 |= CullFlags::SHOW_GI;
        }
        if !args.no_reflections {
            flags.0 |= CullFlags::SKY_REFLECTIONS;
        }
        if !args.no_ray_reflections {
            flags.0 |= CullFlags::RAY_REFLECTIONS;
        }
        if args.show_culled {
            flags.0 |= CullFlags::SHOW_CULLED;
        }
        let mut camera = if args.island.is_some() {
            island_camera(&args)
        } else if args.gallery {
            FlyCamera {
                position: Vec3::new(0.0, 70.0, 230.0),
                pitch: -0.3,
                speed: 40.0,
                ..FlyCamera::default()
            }
        } else {
            // Over the city's south edge, looking north along a street.
            FlyCamera {
                position: Vec3::new(10.0, 45.0, 1260.0),
                pitch: -0.12,
                speed: 80.0,
                ..FlyCamera::default()
            }
        };
        if let Some(v) = &args.view {
            anyhow::ensure!(v.len() == 5, "--view takes x,y,z,yaw,pitch");
            camera.position = Vec3::new(v[0], v[1], v[2]);
            camera.yaw = v[3].to_radians();
            camera.pitch = v[4].to_radians();
        }
        // The probes trace the scene's TLAS (issue #53).
        let probes_on = !args.no_probes;
        anyhow::ensure!(
            (64..=256).contains(&args.probe_rays),
            "--probe-rays takes 64 to 256"
        );
        anyhow::ensure!(
            (1..=forge_render::probes::MAX_CASCADES as u32).contains(&args.probe_cascades),
            "--probe-cascades takes 1 to {}",
            forge_render::probes::MAX_CASCADES
        );
        let probes = if ctx.device.features().ray_query && scene.rays().is_some() {
            let params = ProbeParams {
                rays: args.probe_rays,
                cascades: args.probe_cascades,
                ..ProbeParams::default()
            };
            let probes = Probes::new(&ctx.device, &ctx.shaders, params)?;
            let p = probes.params();
            tracing::info!(
                probes = p.probe_count(),
                cascades = p.cascades,
                spacing_m = p.spacing,
                rays = p.rays,
                mib = %format_args!("{:.1}", probes.bytes() as f64 / f64::from(1 << 20)),
                "diffuse light probes"
            );
            Some(probes)
        } else {
            None
        };
        if let Some(name) = &args.focus {
            let prop = placed
                .iter()
                .find(|p| &p.name == name)
                .ok_or_else(|| anyhow::anyhow!("no prop named {name:?}"))?;
            // Seen from the south-east, a little above, the bounds filling most of the view.
            let (yaw, distance) = (0.6_f32, prop.radius * 2.2);
            let offset = Vec3::new(yaw.sin(), 0.35, yaw.cos()) * distance;
            camera.position = prop.center + offset;
            camera.yaw = yaw;
            camera.pitch = -(0.35_f32).atan();
            camera.speed = prop.radius.max(2.0);
        }
        Ok(Self {
            tonemap: args.tonemap,
            args,
            renderer,
            taa,
            bloom,
            bloom_on,
            atmosphere,
            sky,
            sky_light,
            gtao,
            ao_on,
            probes,
            probes_on,
            probes_live: false,
            day_time: 0.0,
            meter,
            auto_exposure,
            step: 1.0 / 60.0,
            scene,
            camera,
            flags,
            wireframe: false,
            frame: 0,
            stats: Vec::new(),
            gpu_ms: Vec::new(),
            title_updates: 0,
            fly_time: 0.0,
            streaming: Vec::new(),
            frame_ms: Vec::new(),
            run_frame_ms: Vec::new(),
            last_frame: Instant::now(),
            started: None,
            settled: false,
        })
    }

    /// `--day` (issue #57): the sun at `t` of the day (0 sunrise, 0.5 noon, 1 sunset). It rises from
    /// 4° below the eastern horizon to 70° in the south and sets in the west, and its colour is
    /// the sunlight through the air.
    fn set_sun_of_day(&mut self, t: f32) {
        let pi = std::f32::consts::PI;
        let elevation = (-4.0_f32 + 74.0 * (pi * t).sin()).to_radians();
        let azimuth = pi * t;
        self.renderer.sun_dir = Vec3::new(
            elevation.cos() * azimuth.cos(),
            elevation.sin(),
            elevation.cos() * azimuth.sin(),
        );
        let params = &self.atmosphere.params;
        self.renderer.sun_color = Vec3::from(params.transmittance(
            Vec3::new(0.0, params.bottom_radius + 0.05, 0.0),
            self.renderer.sun_dir,
            64,
        ));
    }

    /// Where the camera stands in the world: the scene's origin (`--origin`, issue #93) and its
    /// position in the scene.
    fn camera_position(&self) -> forge_render::CellPos {
        self.scene.origin().offset(self.camera.position)
    }

    fn cull_camera(&self, aspect: f32) -> CullCamera {
        CullCamera::new(
            self.camera.view_rotation(),
            self.camera.projection(aspect),
            self.camera_position(),
            self.camera.near,
        )
    }
}

impl Demo for Gallery {
    fn resized(&mut self, ctx: &mut Context) -> Result<()> {
        self.renderer.resize(ctx.extent())?;
        self.taa.resize(ctx.extent())?;
        self.taa.reset_history();
        Ok(())
    }

    fn key_pressed(&mut self, _ctx: &mut Context, code: KeyCode) {
        match code {
            KeyCode::KeyO => self.flags.toggle(CullFlags::OCCLUSION),
            KeyCode::KeyM => self.flags.toggle(CullFlags::MESHLET_COLORS),
            KeyCode::KeyL => self.flags.toggle(CullFlags::LOD),
            KeyCode::KeyK => self.flags.toggle(CullFlags::LOD_COLORS),
            KeyCode::KeyR => self.args.sw_raster = self.args.sw_raster.next(),
            KeyCode::KeyH => self.flags.toggle(CullFlags::SHOW_RASTER),
            KeyCode::BracketLeft => self.args.lod_error = (self.args.lod_error * 0.5).max(0.125),
            KeyCode::BracketRight => self.args.lod_error = (self.args.lod_error * 2.0).min(16.0),
            KeyCode::Tab => self.wireframe = !self.wireframe,
            KeyCode::KeyG => self.tonemap = self.tonemap.next(),
            KeyCode::KeyB => self.bloom_on = !self.bloom_on,
            KeyCode::KeyI => self.sky_light = !self.sky_light,
            KeyCode::KeyN => self.ao_on = !self.ao_on,
            KeyCode::KeyP => self.probes_on = !self.probes_on,
            KeyCode::KeyV => self.flags.toggle(CullFlags::SHOW_AO),
            KeyCode::KeyU => self.flags.toggle(CullFlags::SHOW_GI),
            KeyCode::KeyF => self.flags.toggle(CullFlags::SKY_REFLECTIONS),
            KeyCode::KeyY => self.flags.toggle(CullFlags::RAY_REFLECTIONS),
            KeyCode::KeyZ => {
                self.renderer.sun_angular_radius = if self.renderer.sun_angular_radius > 0.0 {
                    0.0
                } else {
                    forge_render::starfield::SUN_ANGULAR_RADIUS_1AU
                };
            }
            KeyCode::KeyJ => self.flags.toggle(CullFlags::SHADOWS),
            KeyCode::KeyT => {
                self.taa.enabled = !self.taa.enabled;
                self.taa.reset_history();
            }
            _ => {}
        }
    }

    fn update(&mut self, _ctx: &mut Context, input: &Input, dt: f32) {
        let now = Instant::now();
        let ms = (now - self.last_frame).as_secs_f64() * 1e3;
        self.last_frame = now;
        if self.frame > 0 {
            self.frame_ms.push(ms);
            self.run_frame_ms.push(ms);
        }
        self.step = if self.args.fixed_step { 1.0 / 60.0 } else { dt };
        if let Some(length) = self.args.day {
            self.day_time += self.step;
            self.set_sun_of_day((self.day_time / length.max(1.0)).fract());
        }
        if self.args.fly {
            // Counter-clockwise seen from above, facing along the path, a little down.
            const RADIUS: f32 = 1400.0;
            const SPEED: f32 = 300.0;
            self.fly_time += if self.args.fixed_step { 1.0 / 60.0 } else { dt };
            let angle = self.fly_time * SPEED / RADIUS;
            self.camera.position = Vec3::new(angle.sin() * RADIUS, 140.0, angle.cos() * RADIUS);
            self.camera.yaw = angle - std::f32::consts::FRAC_PI_2;
            self.camera.pitch = -0.15;
        } else if self.args.orbit {
            // Deterministic per frame (not per second) so captures at a frame index match.
            let angle = self.frame as f32 * 0.004;
            let (radius, height, pitch) = if self.args.gallery {
                (230.0 - (self.frame as f32 * 0.1).min(120.0), 45.0, -0.22)
            } else {
                (1500.0, 160.0, -0.12)
            };
            self.camera.position = Vec3::new(angle.sin() * radius, height, angle.cos() * radius);
            self.camera.yaw = angle;
            self.camera.pitch = pitch;
        } else {
            self.camera.update(input, dt);
        }
        self.frame += 1;
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        if let Some(stats) = self.renderer.begin_frame(frame.slot, &mut self.scene)? {
            self.stats.push(stats);
            if let Some(ms) = frame.slot.previous_gpu_ms {
                self.gpu_ms.push(ms);
            }
        }
        if let Some(streaming) = self.scene.streaming() {
            let started = *self.started.get_or_insert_with(Instant::now);
            if self.frame > 8 && streaming.wanted == 0 && streaming.reading == 0 && !self.settled {
                self.settled = true;
                tracing::info!(
                    frame = self.frame,
                    ms = started.elapsed().as_millis(),
                    resident = streaming.resident,
                    "streaming settled"
                );
            }
            ctx.profile.counter(streaming.line());
            self.streaming.push(streaming);
        }
        if let Some(last) = self.stats.last() {
            ctx.profile.counter(format!(
                "drawn through {}: {} instances{}{}, {:.0} k + {:.0} k clusters, {:.2} M triangles, {:.0} k occluded{}; LOD {} at {:.2} px",
                self.renderer.path().name(),
                last.instances_visible,
                last.hidden_note(),
                last.cells_note(),
                f64::from(last.meshlets_pass1) / 1e3,
                f64::from(last.meshlets_pass2) / 1e3,
                f64::from(last.triangles) / 1e6,
                f64::from(last.occluded) / 1e3,
                last.overflow_note(),
                if self.flags.has(CullFlags::LOD) { "on" } else { "off" },
                self.args.lod_error,
            ));
            ctx.profile.counter(last.software_line(self.args.sw_raster));
        }
        if self.frame == 30 {
            // The sky's light once it has been computed, per unit of the sun's illuminance
            // above the air (issue #47): what a roof, a floor and the walls facing towards and
            // away from the sun receive from the sky and the ground, beside the sun's own on
            // the roof.
            let c = self.sky.read_irradiance(&ctx.device)?;
            let sun = self.renderer.sun_dir.normalize();
            let flat = Vec3::new(sun.x, 0.0, sun.z).normalize_or(Vec3::X);
            let luma = |e: Vec3| e.dot(Vec3::new(0.2126, 0.7152, 0.0722));
            let e = |n: Vec3| luma(sh_irradiance(&c, n));
            tracing::info!(
                roof = %format_args!("{:.3}", e(Vec3::Y)),
                floor = %format_args!("{:.3}", e(-Vec3::Y)),
                wall_to_sun = %format_args!("{:.3}", e(flat)),
                wall_away = %format_args!("{:.3}", e(-flat)),
                sun_on_roof = %format_args!("{:.3}", luma(self.renderer.sun_color) * sun.y),
                "sky light, per unit of sun illuminance"
            );
        }
        if self.frame == 120 {
            // What the probes found around the camera (issue #53): per cascade, the probes that
            // light something, and how many of those moved off their cell's centre.
            if let Some(probes) = self.probes.as_ref().filter(|_| self.probes_live) {
                let states = probes.read_states(&ctx.device)?;
                let per = states.len() / probes.params().cascades as usize;
                let (active, moved): (Vec<usize>, Vec<usize>) = states
                    .chunks(per)
                    .map(|c| {
                        let on: Vec<_> = c.iter().filter(|s| s[3] % 4.0 != 1.0).collect();
                        let moved = on
                            .iter()
                            .filter(|s| Vec3::from_slice(&s[..3]).length() > 0.01)
                            .count();
                        (on.len(), moved)
                    })
                    .unzip();
                tracing::info!(
                    per_cascade = per,
                    ?active,
                    ?moved,
                    "probes lighting something"
                );
            }
        }
        // The soft shadows' noise repeats with TAA's jitter (issue #54).
        self.renderer.noise_frame =
            (self.taa.frame_index() % u64::from(self.taa.jitter_phases)) as u32;
        let camera = self.cull_camera(ctx.aspect());
        let extent = ctx.extent();
        // The day's light changes by orders of magnitude: meter it (the histogram of two frames
        // ago); otherwise the fixed exposure of `--ev100`.
        let exposure = if self.args.day.is_some() {
            let histogram = self.meter.take(frame.slot);
            self.auto_exposure.update(histogram.as_ref(), self.step);
            self.auto_exposure.exposure()
        } else {
            exposure_from_ev100(self.args.ev100)
        };
        // Draw jittered into TAA's HDR target, cull with the unjittered camera, resolve
        // through the history and the tone curve into the swapchain.
        let taa_frame = self.taa.begin(
            &mut frame.graph,
            self.camera.projection(ctx.aspect()),
            camera.view_proj,
            camera.position,
            exposure,
        );
        // The camera in the scene frame, where the probes and the rays live (issue #93).
        let camera_in_scene = camera.position.relative_to(self.scene.origin());
        let targets = self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: taa_frame.jittered_projection * self.camera.view_rotation(),
                cull: camera,
                lod_threshold_px: self.args.lod_error,
                draw_jitter: taa_frame.jitter
                    / glam::Vec2::new(extent.width as f32, extent.height as f32),
                flags: self.flags,
                extent,
                wireframe: self.wireframe,
                exposure,
                sw_raster: self.args.sw_raster,
                instance_occlusion: self.args.instance_occlusion,
                instance_cells: !self.args.no_instance_cells,
                sw_raster_area: self.args.sw_raster_area,
            },
        )?;
        // The ground of the city is the surface of an Earth-sized planet: the camera in its
        // frame, in km (`--origin` moves the scene, not the planet). The sky fills what the
        // resolve leaves and hazes the rest (issue #43).
        let view_km = Vec3::new(
            self.camera.position.x * 1e-3,
            self.atmosphere.params.bottom_radius + self.camera.position.y.max(1.0) * 1e-3,
            self.camera.position.z * 1e-3,
        );
        let air =
            self.atmosphere
                .frame(&mut frame.graph, frame.slot, view_km, self.renderer.sun_dir);
        // The sky's tables first: the resolve lights the shaded sides with its irradiance
        // (issue #47).
        let sky = self.sky.tables(
            &mut frame.graph,
            frame.slot,
            &air,
            SkyParams {
                view_proj: taa_frame.jittered_projection * self.camera.view_rotation(),
                camera: Vec3::ZERO,
                sun_dir: self.renderer.sun_dir,
                sun_angular_radius: forge_render::starfield::SUN_ANGULAR_RADIUS_1AU,
                luminance_scale: self.renderer.sun_illuminance * exposure,
                aerial_far_km: 8.0,
            },
            targets.depth,
            taa_frame.color,
            extent,
        );
        // The probes' light in place of the open sky's (issue #53): after the sky's tables,
        // which light their rays' misses, before the resolve.
        let probes = match &mut self.probes {
            Some(probes) if self.probes_on && self.sky_light => {
                if !self.probes_live {
                    probes.reset();
                }
                self.probes_live = true;
                Some(probes.update(
                    &mut frame.graph,
                    frame.slot,
                    self.renderer.frame_address(frame.slot),
                    sky.light,
                    camera_in_scene,
                    self.taa.frame_index() % u64::from(self.taa.jitter_phases),
                ))
            }
            _ => {
                self.probes_live = false;
                None
            }
        };
        // The sky's light, occluded by what the depth shows around each pixel (issue #48).
        let occlusion = (self.sky_light && self.ao_on).then(|| {
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
                sky: self.sky_light.then_some(sky.light),
                occlusion,
                probes,
            },
        );
        self.sky.compose(
            &mut frame.graph,
            &sky,
            targets.depth,
            taa_frame.color,
            extent,
        );
        if self.args.day.is_some() {
            // Meter the finished HDR scene for the exposure of the frames to come.
            self.meter.measure(
                &mut frame.graph,
                frame.slot,
                taa_frame.color,
                extent,
                exposure,
            );
        }
        let motion = self
            .taa
            .motion_vectors(&mut frame.graph, &taa_frame, targets.depth);
        let bloom = self
            .bloom_on
            .then(|| self.bloom.draw(&mut frame.graph, taa_frame.color, extent));
        self.taa.resolve(
            &mut frame.graph,
            &taa_frame,
            targets.depth,
            motion,
            frame.target,
            self.tonemap,
            bloom,
        );
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
        let mut frames = std::mem::take(&mut self.frame_ms);
        let (p50, p99) = (percentile(&mut frames, 0.5), percentile(&mut frames, 0.99));
        let title = format!(
            "forge city-blocks | {} instances, {:.1} M triangles, {:.1} M clusters | {}: drawn {:.0} k instances ({:.0} k hidden; cells {:.1} k listed, {:.1} k hidden whole, {:.2} k opened again), {:.0} k + {:.0} k clusters ({:.0} k in software, {:.1} k left to pass 2; {:.0} k work items, {:.0} k roots), {:.2} M tris | GPU {:.2} ms, frame p50 {p50:.2} p99 {p99:.2} ms",
            self.scene.instance_count,
            self.scene.total_triangles as f64 / 1e6,
            self.scene.instance_meshlets() as f64 / 1e6,
            self.renderer.path().name(),
            mean(|s| s.instances_visible) / 1e3,
            mean(|s| s.instances_occluded) / 1e3,
            mean(|s| s.cells_listed) / 1e3,
            mean(|s| s.cells_deferred) / 1e3,
            mean(|s| s.cells_opened) / 1e3,
            mean(|s| s.meshlets_pass1) / 1e3,
            mean(|s| s.meshlets_pass2) / 1e3,
            mean(|s| s.sw_clusters) / 1e3,
            mean(|s| s.rejected) / 1e3,
            mean(|s| s.work_items) / 1e3,
            mean(|s| s.root_entries) / 1e3,
            mean(|s| s.triangles) / 1e6,
            gpu,
        );
        let title = match self.streaming.last() {
            Some(last) => {
                let frames = self.streaming.len() as f64;
                let uploaded: u32 = self.streaming.iter().map(|s| s.uploaded).sum();
                let wanted = self.streaming.iter().map(|s| s.wanted).max().unwrap_or(0);
                format!(
                    "{title} | pages {} of {} resident, {:.1} uploaded a frame, up to {} wanted",
                    last.resident,
                    last.pool_pages,
                    f64::from(uploaded) / frames,
                    wanted,
                )
            }
            None => title,
        };
        self.streaming.clear();
        self.stats.clear();
        self.gpu_ms.clear();
        self.title_updates += 1;
        if self.title_updates.is_multiple_of(4) {
            tracing::info!("{title}");
        }
        Some(title)
    }
}

impl Drop for Gallery {
    fn drop(&mut self) {
        let mut frames = std::mem::take(&mut self.run_frame_ms);
        if frames.is_empty() {
            return;
        }
        tracing::info!(
            frames = frames.len(),
            p50 = format!("{:.2}", percentile(&mut frames, 0.5)),
            p99 = format!("{:.2}", percentile(&mut frames, 0.99)),
            max = format!("{:.2}", percentile(&mut frames, 1.0)),
            "frame times (ms) over the run"
        );
    }
}

/// The `q` quantile of `values` (sorted in place; 0 when empty).
fn percentile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values
        .get(((values.len().max(1) - 1) as f64 * q).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

/// Cooks (or loads) `props` in parallel on the job system, logging each prop's DAG; returns
/// the meshes in order and the milliseconds they took together.
fn cook_props(props: &[PropSpec], recook: bool, pages_in_memory: bool) -> (Vec<MeshletMesh>, f64) {
    let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
    let cache = root.join("mesh-cache");
    if recook {
        // Stale files would still match their keys: remove this set's before cooking.
        for spec in props {
            let key = forge_geom::cache::key(&spec.key_text());
            let _ = std::fs::remove_file(forge_geom::cache::path(&cache, &spec.name, key));
        }
    }
    let pool = TaskPool::client();
    let mut cooked: Vec<Option<(MeshletMesh, bool, f64)>> = props.iter().map(|_| None).collect();
    pool.scope(|s| {
        for (spec, slot) in props.iter().zip(cooked.iter_mut()) {
            let cache = cache.clone();
            s.spawn(move |_| {
                let spec: &PropSpec = spec;
                let (done, stored) = cook_cached(
                    &cache,
                    &spec.name,
                    &spec.key_text(),
                    spec.cook_options(),
                    pages_in_memory,
                    || spec.generate(),
                );
                if let Err(error) = stored {
                    tracing::warn!(prop = %spec.name, %error, "cooked mesh not cached");
                }
                *slot = Some((done.mesh, done.from_cache, done.ms));
            });
        }
    });
    let mut total_ms = 0.0;
    let meshes = props
        .iter()
        .zip(cooked)
        .map(|(spec, done)| {
            let (mesh, from_cache, ms) = done.expect("prop cooked");
            let dag = mesh.dag_stats();
            tracing::info!(
                prop = %spec.name,
                triangles = mesh.triangle_count,
                clusters = dag.clusters,
                levels = dag.levels,
                roots = dag.roots,
                fill = %format_args!("{:.2}", dag.fill),
                ms = %format_args!("{ms:.0}"),
                from_cache,
                "prop ready"
            );
            total_ms += ms;
            mesh
        })
        .collect();
    (meshes, total_ms)
}

/// The city's materials (issue #20): what each prop is made of, from textures generated at
/// start-up (512 × 512, tileable, with their mips).
struct CityMaterials {
    table: MaterialTable,
    textures: TextureSet,
    /// Row per prop name.
    by_prop: HashMap<&'static str, MaterialId>,
    /// The albedo and normal maps: rock, concrete, brick, grass.
    sets: [(TextureId, TextureId); 4],
}

/// A standard row over a texture set: the textures times a tint each instance mixes from `a`
/// and `b` by its hash, `scale` metres per repeat, a highlight of Blinn-Phong `power`.
fn textured(
    (albedo, normal): (TextureId, TextureId),
    a: [f32; 3],
    b: [f32; 3],
    scale: f32,
    power: f32,
    specular: f32,
) -> RenderLayer {
    RenderLayer {
        color_a: a,
        color_b: b,
        albedo_texture: Some(albedo),
        normal_texture: Some(normal),
        texture_scale: scale,
        roughness: RenderLayer::roughness_for_power(power),
        specular,
        ..RenderLayer::default()
    }
}

impl CityMaterials {
    fn new(device: &Arc<forge_gpu::Device>) -> Result<Self> {
        let start = Instant::now();
        let mut textures = TextureSet::new(device);
        // Generated in parallel: each set is a few hundred thousand noise lookups per map.
        let pool = TaskPool::client();
        let mut sets: [Option<[TextureData; 2]>; 4] = Default::default();
        pool.scope(|s| {
            for (i, slot) in sets.iter_mut().enumerate() {
                s.spawn(move |_| {
                    *slot = Some(match i {
                        0 => textures::rock(11, 512),
                        1 => textures::concrete(12, 512),
                        2 => textures::brick(13, 512),
                        _ => textures::grass(14, 512),
                    });
                });
            }
        });
        let mut ids = Vec::new();
        for set in sets.iter().flatten() {
            ids.push((textures.add(&set[0])?, textures.add(&set[1])?));
        }
        let [rock, concrete, brick, grass] = [ids[0], ids[1], ids[2], ids[3]];
        let sets = [rock, concrete, brick, grass];
        tracing::info!(
            textures = textures.len(),
            mib = textures.bytes() >> 20,
            ms = start.elapsed().as_millis(),
            "city textures"
        );

        let mut table = MaterialTable::new();
        let mut add = |name: &str, layer: RenderLayer| table.add(Material::new(name, layer));
        // A building's window panes are its section 1 (`forge_geom::city::GLASS`): each facade row
        // is followed by the row its windows take.
        let windows = RenderLayer {
            color_a: [0.10, 0.13, 0.16],
            color_b: [0.07, 0.09, 0.12],
            roughness: RenderLayer::roughness_for_power(300.0),
            specular: 0.8,
            // Mildly coated panes (issue #56).
            reflectance: 0.08,
            ..RenderLayer::default()
        };
        let grass = add(
            "grass",
            textured(grass, [1.0; 3], [1.1, 1.05, 0.9], 12.0, 6.0, 0.02),
        );
        let brick_red = add(
            "brick (red)",
            textured(brick, [1.0; 3], [1.1, 0.95, 0.9], 2.0, 10.0, 0.04),
        );
        add("brick (red): windows", windows);
        let brick_brown = add(
            "brick (brown)",
            textured(brick, [0.8, 0.8, 0.85], [0.75, 0.7, 0.65], 2.0, 10.0, 0.04),
        );
        add("brick (brown): windows", windows);
        let concrete_grey = add(
            "concrete",
            textured(
                concrete,
                [0.95, 0.95, 0.93],
                [0.8, 0.8, 0.8],
                4.0,
                12.0,
                0.05,
            ),
        );
        add("concrete: windows", windows);
        let plaster_ochre = add(
            "plaster (ochre)",
            textured(
                concrete,
                [1.1, 0.85, 0.5],
                [1.2, 0.75, 0.45],
                3.0,
                10.0,
                0.04,
            ),
        );
        add("plaster (ochre): windows", windows);
        let plaster_cream = add(
            "plaster (cream)",
            textured(
                concrete,
                [1.25, 1.17, 1.0],
                [1.15, 1.12, 1.05],
                3.0,
                10.0,
                0.04,
            ),
        );
        add("plaster (cream): windows", windows);
        let sandstone = add(
            "sandstone",
            textured(
                concrete,
                [1.05, 0.82, 0.58],
                [0.95, 0.78, 0.6],
                2.5,
                10.0,
                0.04,
            ),
        );
        add("sandstone: windows", windows);
        // A coated curtain wall (issue #56): a mirror of the sky and the city, flat, so no normal
        // map (a bumpy mirror would alias).
        let glass = add(
            "dark glass",
            RenderLayer {
                reflectance: 0.3,
                normal_texture: None,
                roughness: RenderLayer::roughness_for_power(300.0),
                ..textured(
                    concrete,
                    [0.18, 0.21, 0.26],
                    [0.15, 0.17, 0.2],
                    6.0,
                    90.0,
                    0.35,
                )
            },
        );
        add("dark glass: windows", windows);
        let stone = add(
            "stone",
            textured(
                concrete,
                [1.1, 1.05, 0.95],
                [1.0, 0.98, 0.92],
                2.0,
                16.0,
                0.06,
            ),
        );
        let marble = add(
            "marble",
            textured(
                concrete,
                [1.5, 1.47, 1.42],
                [1.45, 1.45, 1.45],
                1.5,
                40.0,
                0.15,
            ),
        );
        // The rock texture averages about 0.37: the ballad's two rock colours over that.
        let rock = add(
            "rock",
            RenderLayer {
                cavity: 0.2,
                ..textured(
                    rock,
                    [1.13, 1.08, 1.03],
                    [1.22, 0.89, 0.68],
                    3.0,
                    14.0,
                    0.06,
                )
            },
        );
        let metal = add(
            "painted metal",
            RenderLayer {
                color_a: [0.04, 0.05, 0.045],
                color_b: [0.05, 0.05, 0.05],
                roughness: RenderLayer::roughness_for_power(60.0),
                specular: 0.25,
                ..RenderLayer::default()
            },
        );
        let by_prop = HashMap::from([
            ("terrain", grass),
            ("house-narrow", brick_red),
            ("house-wide", plaster_ochre),
            ("corner-block", brick_brown),
            ("terrace", brick_red),
            ("apartments", plaster_cream),
            ("office", concrete_grey),
            ("hotel", sandstone),
            ("warehouse", concrete_grey),
            ("school", brick_brown),
            ("clinic", plaster_cream),
            ("tower-slim", glass),
            ("tower-wide", concrete_grey),
            ("boulder-1", rock),
            ("boulder-2", rock),
            ("boulder-3", rock),
            ("rubble-1", rock),
            ("rubble-2", rock),
            ("column", marble),
            ("fountain", stone),
            ("lamp-post", metal),
        ]);
        Ok(Self {
            table,
            textures,
            by_prop,
            sets,
        })
    }

    /// The ground in layers (issue #42): uploads `layers` (`texels` a side over `size` metres,
    /// `placement::ground_layers`) and adds the layered row and one row per layer after it,
    /// in `placement::layer` order. The terrain takes the layered row.
    fn ground(&mut self, layers: &[u8], texels: u32, size: f32) -> Result<MaterialId> {
        let map = self
            .textures
            .add_layer_map("ground layers", texels, texels, layers)?;
        let [rock, concrete, brick, grass] = self.sets;
        let ground = self.table.add(Material::new(
            "ground",
            RenderLayer {
                class: ShadingClass::Layered,
                albedo_texture: Some(map),
                texture_scale: size,
                ..RenderLayer::default()
            },
        ));
        let rows = [
            (
                "ground: grass",
                textured(grass, [1.0; 3], [1.1, 1.05, 0.9], 12.0, 6.0, 0.02),
            ),
            (
                "ground: asphalt",
                textured(
                    concrete,
                    [0.2, 0.2, 0.21],
                    [0.2, 0.2, 0.21],
                    5.0,
                    20.0,
                    0.05,
                ),
            ),
            (
                "ground: sidewalk",
                textured(
                    concrete,
                    [0.9, 0.88, 0.85],
                    [0.9, 0.88, 0.85],
                    1.5,
                    12.0,
                    0.04,
                ),
            ),
            (
                "ground: paving",
                textured(
                    brick,
                    [0.78, 0.75, 0.72],
                    [0.78, 0.75, 0.72],
                    1.2,
                    12.0,
                    0.05,
                ),
            ),
            (
                "ground: rock",
                textured(
                    rock,
                    [1.13, 1.08, 1.03],
                    [1.13, 1.08, 1.03],
                    6.0,
                    14.0,
                    0.06,
                ),
            ),
        ];
        assert_eq!(rows.len(), usize::from(placement::layer::COUNT));
        for (name, layer) in rows {
            self.table.add(Material::new(name, layer));
        }
        self.by_prop.insert("terrain", ground);
        Ok(ground)
    }

    /// The island's ground (`docs/demos/island.md`, #96): its layer map (`island_layer`) and a
    /// row per layer after the layered row, for a tropical island rather than the city's
    /// hills: a deeper green, sand on the beaches, wet sand under the sea, dark volcanic rock
    /// on the steep ground. Then the sea's row, calm water, smooth and dark, for the plane that
    /// stands in for the sea until the water is drawn (D-038, `sea_prop`).
    fn island_ground(&mut self, layers: &[u8], texels: u32, size: f32) -> Result<MaterialId> {
        let map = self
            .textures
            .add_layer_map("island layers", texels, texels, layers)?;
        let [rock, concrete, _, grass] = self.sets;
        let ground = self.table.add(Material::new(
            "island ground",
            RenderLayer {
                class: ShadingClass::Layered,
                albedo_texture: Some(map),
                texture_scale: size,
                ..RenderLayer::default()
            },
        ));
        let rows = [
            (
                "island: grass",
                textured(
                    grass,
                    [0.72, 0.9, 0.55],
                    [0.82, 0.98, 0.62],
                    12.0,
                    6.0,
                    0.02,
                ),
            ),
            (
                "island: sand",
                textured(
                    concrete,
                    [0.86, 0.76, 0.56],
                    [0.94, 0.85, 0.66],
                    2.0,
                    10.0,
                    0.04,
                ),
            ),
            (
                "island: seabed",
                textured(
                    concrete,
                    [0.5, 0.45, 0.34],
                    [0.56, 0.5, 0.38],
                    2.0,
                    10.0,
                    0.04,
                ),
            ),
            (
                "island: rock",
                textured(rock, [0.3, 0.3, 0.29], [0.38, 0.37, 0.35], 6.0, 14.0, 0.05),
            ),
            (
                // Water over a dark bed, as the sea's row: the rivers' stand-in (D-038).
                "island: stream",
                RenderLayer {
                    color_a: [0.02, 0.045, 0.05],
                    color_b: [0.025, 0.05, 0.055],
                    roughness: RenderLayer::roughness_for_power(200.0),
                    specular: 0.5,
                    reflectance: 0.02,
                    ..RenderLayer::default()
                },
            ),
        ];
        assert_eq!(rows.len(), usize::from(island_layer::COUNT));
        for (name, layer) in rows {
            self.table.add(Material::new(name, layer));
        }
        self.by_prop.insert("island", ground);
        let sea = self.table.add(Material::new(
            "island: sea",
            RenderLayer {
                color_a: [0.015, 0.05, 0.07],
                color_b: [0.02, 0.06, 0.08],
                roughness: RenderLayer::roughness_for_power(400.0),
                specular: 0.5,
                reflectance: 0.02,
                ..RenderLayer::default()
            },
        ));
        self.by_prop.insert("sea", sea);
        // The island's boulders and rubble: the same dark volcanic rock, not the city's pale
        // stone, with the city rock's cavity darkening.
        let boulders = self.table.add(Material::new(
            "island: boulders",
            RenderLayer {
                cavity: 0.2,
                ..textured(
                    rock,
                    [0.34, 0.33, 0.31],
                    [0.42, 0.39, 0.35],
                    3.0,
                    14.0,
                    0.06,
                )
            },
        ));
        for prop in [
            "boulder-1",
            "boulder-2",
            "boulder-3",
            "rubble-1",
            "rubble-2",
        ] {
            self.by_prop.insert(prop, boulders);
        }
        Ok(ground)
    }

    /// The row `prop` is made of (the default grey for a prop the table does not know).
    fn of(&self, prop: &str) -> MaterialId {
        self.by_prop
            .get(prop)
            .copied()
            .unwrap_or(MaterialTable::DEFAULT)
    }

    /// Gives every mesh its prop's row and hands the table and the textures to the scene.
    fn apply(self, builder: &mut MeshletSceneBuilder, props: &[PropSpec], ids: &[MeshId]) {
        for (spec, &id) in props.iter().zip(ids) {
            builder.set_mesh_material(id, self.of(&spec.name));
        }
        builder.set_materials(&self.table, Some(self.textures));
    }
}

/// The props a run draws (the city's twenty and its terrain, or the gallery's twenty), cooked
/// or loaded from the cache, and the milliseconds the props took summed.
struct Cooked {
    meshes: Vec<MeshletMesh>,
    ms: f64,
}

/// Cooks (or loads) the props of this run: the start-up's CPU work, which runs behind the
/// loading screen (issue #25).
fn cook(args: &Args) -> Cooked {
    let props = if args.island.is_some() {
        // The island, the sea around it and the rocks on it (`docs/demos/island.md`).
        island_props(args)
    } else {
        let mut props = city_props();
        if !args.gallery {
            props.push(PropSpec {
                name: "terrain".to_owned(),
                kind: PropKind::Terrain(Terrain::city()),
            });
        }
        props
    };
    // The streamed city keeps its pages on the GPU only (issue #36).
    let pages_in_memory = args.gallery || args.stream_pool == 0;
    let (meshes, ms) = cook_props(&props, args.recook, pages_in_memory);
    Cooked { meshes, ms }
}

/// The island's ground layers (`CityMaterials::island_ground`, `forge_procgen::slope_layers`).
mod island_layer {
    /// Grass, on the gentle ground above the beaches.
    pub const GRASS: u8 = 0;
    /// Sand, on the land's first metres above the sea.
    pub const SAND: u8 = 1;
    /// The sea floor, under the sea's plane (wet sand).
    pub const SEABED: u8 = 2;
    /// Rock, where the ground is steep.
    pub const ROCK: u8 = 3;
    /// A river, painted over the others at its width (`forge_procgen::paint_rivers`).
    pub const STREAM: u8 = 4;
    /// How many layers there are.
    pub const COUNT: u8 = 5;
}

/// The island's generation settings from the arguments (`--island`, `--island-spacing`,
/// `--island-steps`): the 16 km island of `forge-procgen` with its default erosion.
fn island_settings(args: &Args) -> (IslandParams, ErosionParams) {
    let seed = forge_core::Seed::new(args.island.unwrap_or(7));
    let mut params = IslandParams::island_16km(seed, args.island_spacing);
    if let Some(from) = &args.island_wind {
        params.wind = forge_procgen::Wind::from_compass(from, args.island_rain_contrast);
        if params.wind.is_none() {
            tracing::warn!(wind = %from, "unknown wind origin (n, ne, e, se, s, sw, w, nw): the rain stays flat");
        }
    }
    let erosion = ErosionParams {
        steps: args.island_steps,
        ..ErosionParams::island()
    };
    (params, erosion)
}

/// The island's heightfield, generated once and kept in `mesh-cache/` beside the cooked
/// meshes (`forge_procgen::cached_island`).
fn island_heights(args: &Args) -> Field2<f32> {
    let (params, erosion) = island_settings(args);
    let dir = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR")).join("mesh-cache");
    let start = Instant::now();
    let pool = TaskPool::client();
    let (height, from_cache) = forge_procgen::cached_island(&dir, &params, &erosion, &pool)
        .expect("the island's cache file");
    // The sea floor under the flat sea (#96): the sea's plane then meets the ground along the
    // coast, between the samples.
    let mut height = height;
    let coast = forge_procgen::coast_distance(&height, 0.0, &pool);
    forge_procgen::sea_floor(&mut height, &coast, 0.0, SEA_FLOOR.0, SEA_FLOOR.1);
    let (lo, hi) = height.min_max();
    tracing::info!(
        seed = args.island.unwrap_or(7),
        samples = height.size,
        spacing_m = height.spacing,
        from_cache,
        ms = start.elapsed().as_millis(),
        height_m = %format_args!("{lo:.0}-{hi:.0}"),
        "island heightfield"
    );
    height
}

/// The island's sea floor (`forge_procgen::sea_floor`): metres of depth it levels off at, and
/// the metres from the coast that set its slope (60 over 1 500: 4 % at the shore).
const SEA_FLOOR: (f32, f32) = (60.0, 1500.0);

/// The island as a prop (on its layered ground, `CityMaterials::island_ground`), its samples
/// generated only when the cooked mesh is not in the cache. Named `island`, not `terrain`:
/// the cache keeps one file per name, and the city's ground and the island evicted each other.
fn island_prop(args: &Args) -> PropSpec {
    let (params, erosion) = island_settings(args);
    let for_source = args.clone();
    PropSpec {
        name: "island".to_owned(),
        kind: PropKind::Heightfield(Heightfield {
            key: format!(
                "{}, sea floor {} m over {} m",
                forge_procgen::island::island_key(&params, &erosion),
                SEA_FLOOR.0,
                SEA_FLOOR.1
            ),
            samples: params.size,
            spacing: params.spacing as f32,
            source: std::sync::Arc::new(move || island_heights(&for_source).data),
        }),
    }
}

/// The island's first view (#96): on its south coast looking inland, 25 m over the water
/// 150 m off the beach due south of the centre, found in the field (the first sample above
/// 1 m walking north from the domain's south edge), so it holds for any seed. From farther out,
/// the whole island: `--view 0,300,6800,0,-0.08`.
fn island_camera(args: &Args) -> FlyCamera {
    let height = island_heights(args);
    let n = height.size;
    let half = height.extent() * 0.5;
    let beach = (0..n)
        .rev()
        .find(|&j| height.get(n / 2, j) > 1.0)
        .map_or(0.0, |j| f64::from(j) * height.spacing - half);
    FlyCamera {
        position: Vec3::new(0.0, 25.0, (beach + 150.0) as f32),
        pitch: 0.03,
        speed: 60.0,
        ..FlyCamera::default()
    }
}

/// The island's props, in the order `build_island` reads them: the island, the sea around it,
/// and the city's boulders and rubble for its rocks (the same cache files as the city's).
fn island_props(args: &Args) -> Vec<PropSpec> {
    let mut props = vec![island_prop(args), sea_prop()];
    props.extend(
        city_props()
            .into_iter()
            .filter(|p| matches!(p.kind, PropKind::Boulder { .. } | PropKind::Rubble { .. })),
    );
    props
}

/// The sea's stand-in until the water pass (D-038, #96): one opaque plane at 0 m, 262 km
/// across, over the island's sea floor and out to the horizon, on the sea's row. The coast is
/// where it meets the ground.
fn sea_prop() -> PropSpec {
    const SAMPLES: u32 = 33;
    PropSpec {
        name: "sea".to_owned(),
        kind: PropKind::Heightfield(Heightfield {
            key: "a flat sea at 0 m".to_owned(),
            samples: SAMPLES,
            spacing: 8192.0,
            source: Arc::new(|| vec![0.0; (SAMPLES * SAMPLES) as usize]),
        }),
    }
}

/// The island (`docs/demos/island.md`): its heightfield cooked (or loaded) as the one
/// instance of the scene, on the ground's layered material with rock where the ground is
/// steep or high and grass elsewhere.
fn build_island(ctx: &Context, args: &Args, cooked: Cooked) -> Result<MeshletScene> {
    let start = Instant::now();
    let props = island_props(args);
    let streamed = args.stream_pool > 0;
    let (meshes, cook_ms) = (cooked.meshes, cooked.ms);
    let mut builder = MeshletSceneBuilder::new();
    let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    // The layers from the field: a texel every 4 m over the 16 km (stage 6's first rule).
    let height = island_heights(args);
    let extent = height.extent() as f32;
    let texels = 4096;
    let layers_start = Instant::now();
    let mut layers = forge_procgen::slope_layers(
        &height,
        &forge_procgen::LayerRule {
            grass: island_layer::GRASS,
            rock: island_layer::ROCK,
            rock_slope: 0.45,
            // Green to the peaks, as on a tropical island: rock where it is steep.
            rock_above: f32::INFINITY,
            // The slope over 8 m whatever the spacing, so 4 m draws the rock 8 m draws.
            slope_over: 8.0,
            shore: Some(forge_procgen::Shore {
                sea: island_layer::SEABED,
                sand: island_layer::SAND,
                sand_below: 2.5,
            }),
        },
        texels,
    );
    tracing::info!(
        texels,
        ms = layers_start.elapsed().as_millis(),
        "island layer map"
    );
    // The rivers of stage 4 over it: the drawn field's drainage, the rivers above 0.5 km² of
    // catchment (as `genesis` traces them), painted at their width, 8 m at least (two texels).
    let rivers_start = Instant::now();
    let flow = forge_procgen::drain(&height, 0.0, &TaskPool::client());
    let min_area = (500_000.0 / (height.spacing * height.spacing)) as u32 + 1;
    let rivers = forge_procgen::trace_rivers(&height, &flow, min_area);
    let painted = forge_procgen::paint_rivers(
        &mut layers,
        &rivers,
        height.spacing,
        island_layer::STREAM,
        8.0,
    );
    tracing::info!(
        rivers = rivers.rivers.len(),
        texels = painted,
        ms = rivers_start.elapsed().as_millis(),
        "island rivers"
    );
    builder.set_ray_traced(!args.no_shadows);
    let mut materials = CityMaterials::new(&ctx.device)?;
    materials.island_ground(&layers.data, texels, extent)?;
    materials.apply(&mut builder, &props, &ids);
    let mut layout = CityLayout::island(args.instances.unwrap_or(300_000));
    layout.origin = scene_origin(args);
    builder.set_origin(layout.origin);
    builder.add_instance(ids[0], Mat4::IDENTITY);
    builder.add_instance(ids[1], Mat4::IDENTITY);
    // The rocks: the GPU placement over the island's own heights (`placement::RockRule::Land`).
    let rocks: Vec<MeshId> = ids[2..].to_vec();
    let meshes = CityMeshes {
        buildings: Vec::new(),
        rocks: rocks.clone(),
        // No city: none of these is placed.
        lamp: rocks[0],
        fountain: rocks[0],
        column: rocks[0],
    };
    let first = builder.reserve_instances(&placement::mesh_counts(&layout, &meshes));
    let ground = Ground {
        heights: &height.data,
        samples: height.size,
        spacing: height.spacing as f32,
    };
    let residency = if streamed {
        Residency::Streamed(StreamingConfig::from_mib(
            args.stream_pool,
            args.stream_upload,
        ))
    } else {
        Residency::All
    };
    let mut scene = builder.build_with(&ctx.device, residency)?;
    tracing::info!(
        pages = scene.page_count,
        mib = (u64::from(scene.page_count) * forge_geom::PAGE_SIZE as u64) >> 20,
        streamed,
        "cluster pages"
    );
    let report = placement::place(
        &ctx.device,
        &ctx.shaders,
        &scene,
        first,
        &layout,
        &meshes,
        &ground,
    )?;
    tracing::info!(
        rocks = report.placed,
        ms = %format_args!("{:.1}", report.ms),
        checksum = %format_args!("{:016x}", report.checksum),
        matches_cpu_mirror = report.matches_mirror,
        "island rocks placed"
    );
    // The instance culls read the sorted table by cells of 64 (issue #38).
    scene.build_cells(&ctx.device, &ctx.shaders)?;
    // The sun's shadows and the probes trace against the island and its rocks (#45, #53).
    scene.build_tlas(&ctx.device, &ctx.shaders)?;
    log_rays(&scene);
    tracing::info!(
        origin_m = args.origin,
        f32_spacing_m = forge_render::precision::ulp(args.origin.abs() + 0.5 * extent),
        "the scene's offset from the world's origin"
    );
    tracing::info!(
        triangles = scene.total_triangles,
        clusters = scene.instance_meshlets(),
        cook_ms = %format_args!("{cook_ms:.0}"),
        wall_ms = start.elapsed().as_millis(),
        "island ready"
    );
    Ok(scene)
}

/// The city: the terrain and the twenty props cooked (or loaded), the terrain placed once
/// at the origin and `args.instances` props placed over it by the GPU.
fn build_city(ctx: &Context, args: &Args, cooked: Cooked) -> Result<MeshletScene> {
    let start = Instant::now();
    let terrain = Terrain::city();
    let mut props = city_props();
    props.push(PropSpec {
        name: "terrain".to_owned(),
        kind: PropKind::Terrain(terrain.clone()),
    });
    let streamed = args.stream_pool > 0;
    let (meshes, cook_ms) = (cooked.meshes, cooked.ms);
    let mut builder = MeshletSceneBuilder::new();
    let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    let mut layout = CityLayout::city(args.instances.unwrap_or(1_000_000));
    // The city stands `--origin` from the world's origin (issue #93): the scene's origin, an
    // integer cell and an offset, from which each instance gets its own cell; the layout stays
    // around its own centre.
    layout.origin = scene_origin(args);
    builder.set_origin(layout.origin);
    // The heightfield the terrain mesh was sampled from.
    let heights_start = std::time::Instant::now();
    let heights = parallel_heights(&terrain);
    tracing::info!(
        samples = heights.len(),
        ms = heights_start.elapsed().as_millis(),
        "terrain heights"
    );
    let ground = Ground {
        heights: &heights,
        samples: terrain.samples(),
        spacing: terrain.spacing,
    };
    // The ground's layers, a metre a texel: streets, sidewalks, plazas, lots, rock on the
    // steep hills (issue #42).
    let layers_start = std::time::Instant::now();
    let texels = terrain.size as u32;
    let layers = placement::ground_layers(&layout, &ground, texels);
    tracing::info!(
        texels,
        ms = layers_start.elapsed().as_millis(),
        "ground layers"
    );
    builder.set_ray_traced(!args.no_shadows);
    let mut materials = CityMaterials::new(&ctx.device)?;
    materials.ground(&layers, texels, terrain.size)?;
    materials.apply(&mut builder, &props, &ids);
    let id = |name: &str| ids[props.iter().position(|p| p.name == name).expect("prop")];
    let terrain_id = id("terrain");
    builder.add_instance(terrain_id, Mat4::IDENTITY);
    let city = CityMeshes {
        buildings: props
            .iter()
            .zip(&ids)
            .filter(|(p, _)| matches!(p.kind, PropKind::Building(_)))
            .map(|(_, &id)| id)
            .collect(),
        rocks: props
            .iter()
            .zip(&ids)
            .filter(|(p, _)| matches!(p.kind, PropKind::Boulder { .. } | PropKind::Rubble { .. }))
            .map(|(_, &id)| id)
            .collect(),
        lamp: id("lamp-post"),
        fountain: id("fountain"),
        column: id("column"),
    };
    let first = builder.reserve_instances(&placement::mesh_counts(&layout, &city));
    let residency = if streamed {
        Residency::Streamed(StreamingConfig::from_mib(
            args.stream_pool,
            args.stream_upload,
        ))
    } else {
        Residency::All
    };
    let mut scene = builder.build_with(&ctx.device, residency)?;
    tracing::info!(
        pages = scene.page_count,
        mib = (u64::from(scene.page_count) * forge_geom::PAGE_SIZE as u64) >> 20,
        streamed,
        "cluster pages"
    );
    let report = placement::place(
        &ctx.device,
        &ctx.shaders,
        &scene,
        first,
        &layout,
        &city,
        &ground,
    )?;
    let counts = layout.counts();
    tracing::info!(
        placed = report.placed,
        buildings = counts.buildings,
        lamps = counts.lamps,
        plaza_props = counts.plaza_slots,
        rocks = counts.rocks,
        ms = %format_args!("{:.1}", report.ms),
        checksum = %format_args!("{:016x}", report.checksum),
        matches_cpu_mirror = report.matches_mirror,
        "instances placed"
    );
    if !report.matches_mirror {
        tracing::warn!("the placed meshes differ from the CPU mirror: the scene's counts are off");
    }
    // How finely an f32 resolves a position at the terrain's far edge (issue #93).
    tracing::info!(
        origin_m = args.origin,
        f32_spacing_m = forge_render::precision::ulp(args.origin.abs() + ground.half_size()),
        "the scene's offset from the world's origin"
    );
    // The instance culls read the sorted table by cells of 64 (issue #38).
    scene.build_cells(&ctx.device, &ctx.shaders)?;
    // The sun's shadows trace against every placed instance (issue #45).
    scene.build_tlas(&ctx.device, &ctx.shaders)?;
    log_rays(&scene);
    tracing::info!(
        instances = scene.instance_count,
        triangles = scene.total_triangles,
        clusters = scene.instance_meshlets(),
        cook_ms = %format_args!("{cook_ms:.0}"),
        wall_ms = start.elapsed().as_millis(),
        "city ready"
    );
    Ok(scene)
}

/// Cooks (or loads) every prop of the city set in parallel and lays one of each out on a
/// grid, `SPACING` metres apart.
fn build_gallery(
    ctx: &Context,
    args: &Args,
    cooked: Cooked,
) -> Result<(MeshletScene, Vec<Placed>)> {
    let start = Instant::now();
    let props = city_props();
    let (meshes, total_ms) = (cooked.meshes, cooked.ms);
    let mut builder = MeshletSceneBuilder::new();
    let mut placed = Vec::with_capacity(props.len());
    let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    CityMaterials::new(&ctx.device)?.apply(&mut builder, &props, &ids);
    builder.set_ray_traced(!args.no_shadows);
    builder.set_origin(scene_origin(args));
    for (i, (spec, mesh)) in props.iter().zip(&meshes).enumerate() {
        let id = ids[i];
        let (column, row) = (i as u32 % COLUMNS, i as u32 / COLUMNS);
        let position = Vec3::new(
            (column as f32 - (COLUMNS - 1) as f32 * 0.5) * SPACING,
            0.0,
            (row as f32 - 1.5) * SPACING,
        );
        builder.add_instance(id, Mat4::from_translation(position));
        placed.push(Placed {
            name: spec.name.clone(),
            center: position + Vec3::from(mesh.center),
            radius: mesh.radius,
        });
    }
    let mut scene = builder.build(&ctx.device)?;
    scene.build_tlas(&ctx.device, &ctx.shaders)?;
    tracing::info!(
        props = scene.instance_count,
        triangles = scene.total_triangles,
        clusters = scene.instance_meshlets(),
        prop_ms_sum = %format_args!("{total_ms:.0}"),
        wall_ms = start.elapsed().as_millis(),
        "gallery ready"
    );
    Ok((scene, placed))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = AppConfig {
        title: "forge city-blocks".into(),
        vsync: args.vsync,
        validate: args.validate,
        frame_limit: args.frames,
        capture: args.capture.clone().map(|p| (p, args.capture_frame)),
        capture_every: None,
        overlay: if args.overlay { Some(true) } else { None },
        force_fallback: args.force_fallback,
        width: args.width,
        height: args.height,
        ..AppConfig::default()
    };
    // The props cook (or load from the cache) behind the loading screen (issue #25).
    forge_app::run_loading(config, move || {
        let cooked = cook(&args);
        let finish: Finish<Gallery> = Box::new(move |ctx| Gallery::new(ctx, args, cooked));
        Ok(finish)
    })
}

/// The acceleration structures' size, build time and how far their cuts may stand off the
/// drawn surfaces (issue #45).
fn log_rays(scene: &MeshletScene) {
    if let Some(rays) = scene.rays() {
        tracing::info!(
            blas_triangles = rays.triangles,
            max_cut_error = %format_args!("{:.3}", rays.max_cut_error),
            mib = rays.bytes() >> 20,
            hit_data_mib = rays.hit_bytes >> 20,
            blas_ms = %format_args!("{:.0}", rays.blas_ms),
            tlas_ms = %format_args!("{:.0}", rays.tlas_ms),
            "acceleration structures"
        );
    }
}

/// The scene's origin (`--origin`, issue #93): the same distance along every axis, split into
/// an integer cell and an offset from an `f64`, so the split is exact.
fn scene_origin(args: &Args) -> forge_render::CellPos {
    forge_render::CellPos::from_f64(glam::DVec3::splat(f64::from(args.origin)))
}

/// [`Terrain::heights`] on the job system, 64 rows to a job.
fn parallel_heights(terrain: &Terrain) -> Vec<f32> {
    let n = terrain.samples() as usize;
    let mut heights = vec![0.0; n * n];
    TaskPool::client().scope(|s| {
        for (chunk, rows) in heights.chunks_mut(64 * n).enumerate() {
            s.spawn(move |_| terrain.heights_into((chunk * 64) as u32, rows));
        }
    });
    heights
}

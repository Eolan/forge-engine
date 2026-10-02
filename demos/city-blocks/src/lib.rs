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
//! reflections, Y mirror rays in the glass and the water, Z soft or hard shadows, Tab wireframe,
//! G tone curve, Esc quit.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches, Parser};
use forge_app::{AppConfig, Context, Demo, Finish, FlyCamera, FrameInfo, HdrMode, Input};
use forge_core::material::{
    LayerContour, Material, MaterialId, MaterialTable, RenderLayer, ShadingClass, TextureId,
};
use forge_geom::MeshletMesh;
use forge_geom::cache::cook_cached;
use forge_geom::city::{
    CellWindow, Heightfield, HeightfieldDetail, Lathe, PropKind, PropSpec, Terrain, city_props,
};
use forge_geom::stone::{Stone, StoneShape};
use forge_procgen::{
    ErosionParams, Field2, IslandParams, Ocean, OceanParams, ShoreProfile, ShoreTrain,
};
use forge_render::material::TextureSet;
use forge_render::meshlet::{DrawParams, MeshId};
use forge_render::placement::{self, CityLayout, CityMeshes, Ground};
use forge_render::textures::{self, TextureData};
use forge_render::{
    AmbientLight, Atmosphere, AtmosphereParams, AutoExposure, Bloom, CullCamera, CullFlags,
    FrameStats, GroundSky, Gtao, GtaoParams, HdrOutput, LuminanceMeter, MAX_FLOATERS, MAX_WAKES,
    MeshletRenderer, MeshletScene, MeshletSceneBuilder, MoverTransform, ProbeParams, Probes,
    Residency, SkyParams, SplashParams, SplashSource, StartView, StreamingConfig, StreamingStats,
    SwRaster, Taa, Tonemap, WaterCascadeDesc, WaterCascades, WaterCaustics, WaterFloater,
    WaterLake, WaterMouth, WaterRiverPoint, WaterShore, WaterShoreTrain, WaterSplashes, WaterStone,
    WaterSurface, WaterSurfaceParams, WaterWake, WaterWakes, exposure_from_ev100, sh_irradiance,
};
use forge_task::TaskPool;
use glam::{Mat4, Quat, Vec2, Vec3};
use winit::keyboard::KeyCode;

mod island_demo;

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
    /// Also capture every N-th frame (`<capture stem>-NNNNN.png`), for a sequence (LOD pops,
    /// `imgdiff --then`).
    #[arg(long)]
    capture_every: Option<u64>,
    /// The island's stones' weight of their normals when they are cooked (#131), per metre of
    /// their size: metres of error per unit of normal change and metre (`CookOptions`; the
    /// asteroids' `--lod-normals`). 0 cooks them by their geometry alone, as before; 2 halves
    /// the pops of 0 along a glide past the granite's stones.
    #[arg(long, default_value_t = 2.0)]
    stone_normals: f32,
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
    /// HDR output: off, hdr10, scrgb or offscreen (F2 switches it at run time, F3 steps the
    /// peak, F5 opens the calibration pages). HDR10 and scRGB need the OS to show the display in
    /// HDR; offscreen previews an HDR10 image on any monitor. ACES 2.0 (G) is the curve made for
    /// it.
    #[arg(long, default_value = "off")]
    hdr: HdrMode,
    /// The paper-white offset in HDR: stops added to the scene before ACES 2.0 (0, the
    /// Academy's look).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    hdr_stops: f32,
    /// The UI's white in HDR, in nits (by default the calibration's, else the OS's SDR white,
    /// else 203).
    #[arg(long)]
    hdr_ui_white: Option<f32>,
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
    /// Metres between the island's drawn samples (#106): with less than the field's spacing,
    /// the ground is drawn on the field's cubic carved by the rivers' channels (2: 8 193²
    /// samples, 143 M triangles with the amplification's detail) rather than on the field's
    /// cells (8); the channels' and shores' cells stay at a metre.
    #[arg(long, default_value_t = 2.0)]
    island_drawn: f64,
    /// How much of the amplification's detail the ground drawn finer than the field takes
    /// (#106, `forge_procgen::amplify` at each halving of the spacing): 1 all of it away from
    /// the water, 0 none (the field's cubic).
    #[arg(long, default_value_t = 1.0)]
    island_detail: f32,
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
    /// The catchment, hectares, from which a channel of the island's erosion carries away all
    /// the hillslopes shed into it (#109); 0 lets the diffusion raise every cell.
    #[arg(long, default_value_t = ErosionParams::island().channel_area / 10_000.0)]
    island_channel_ha: f64,
    /// The island's coastal plain (D-041): the share of its radius inland over which the
    /// uplift stays low (0: the hills rise from the coast).
    #[arg(long)]
    island_plain: Option<f64>,
    /// The uplift on the island's coastal plain, a share of the hills' starting rate.
    #[arg(long)]
    island_plain_uplift: Option<f64>,
    /// How far the coastal plain's width wanders along the coast.
    #[arg(long)]
    island_plain_wander: Option<f64>,
    /// The island's large basins (D-041's scale, #123): how many trunk valleys its uplift is
    /// lowered along, a lake's bowl on each; 0 lifts the dome of before, rivers running out on
    /// every side.
    #[arg(long)]
    island_basins: Option<u32>,
    /// The alluvium's grade on the island (#123): every sample at least this many metres over
    /// the sea per metre of its way down to it, so a large river's lower course falls to its
    /// mouth; 0 leaves the erosion's field (with `--island-basins 0`, the island of before).
    #[arg(long)]
    island_grade: Option<f64>,
    /// The island's rivers sized by D-041's regional curves with this exaggeration `k`: a river
    /// `k · 2.7 (A/km²)^0.37` m wide, `1.5 · 0.3 (A/km²)^0.21` m deep. 0 sizes them by the
    /// catchment's square root (5 m at a square kilometre), as before D-041.
    #[arg(long, default_value_t = forge_procgen::RibbonParams::island().regional.map_or(0.0, |(k, _)| k))]
    river_k: f64,
    /// Leave the island's rivers in the valleys the erosion cut: no floor for their water, no
    /// bench or floodplain (#116, D-041).
    #[arg(long)]
    no_valleys: bool,
    /// Let the island's steep rivers fall evenly, without their steps and pools (#122, D-041).
    #[arg(long)]
    no_steps: bool,
    /// Size the island's small rivers as its large ones (`--river-k` from the smallest),
    /// instead of brooks of nature's size easing to it by 3 km² of catchment (#123, D-041).
    #[arg(long)]
    no_brooks: bool,
    /// Run the island's rivers into their lakes as they come, without their deltas: no easing
    /// to the lake's level, no widening, no fan on the lake's floor (#120, D-041).
    #[arg(long)]
    no_deltas: bool,
    /// Leave the shallow arms past the lakes' outlets flooded: no sill rising over the lake's
    /// level there, the lake's water not trimmed off them (#120).
    #[arg(long)]
    no_sills: bool,
    /// Run the island's large rivers into the sea in one channel, without the bars of sand their
    /// water splits around in their mouths (#127, D-041).
    #[arg(long)]
    no_bars: bool,
    /// Leave every beach of the island pale sand: no shingle on the headlands and under steep
    /// land (#128).
    #[arg(long)]
    no_beach_types: bool,
    /// Leave the island's rock one dark grey: no granite in the hills, no limestone on the low
    /// ground and the sea cliffs, no karst (#129, D-042).
    #[arg(long)]
    no_rock_types: bool,
    /// Strew the island's rocks as before (#130): 300 000 of the city's boulders and rubble,
    /// more on the steeper ground, in one dark grey, instead of fewer stones of the island's own
    /// granite and limestone where rocks gather.
    #[arg(long)]
    no_rock_sites: bool,
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
    /// Draw the island without its water (issue #105, D-038): the stand-in sea's opaque plane,
    /// and the rivers and lakes painted into the ground's layers.
    #[arg(long)]
    no_water: bool,
    /// The island's water, drawn by default since 2026-10-01: kept so older commands run.
    #[arg(long = "water", hide = true, conflicts_with = "no_water")]
    legacy_water: bool,
    /// Light the sea floor without the waves' caustics (#108).
    #[arg(long)]
    no_caustics: bool,
    /// Moving geometry (#79): this many barrels drifting down the island's largest rivers,
    /// their transforms written every frame. None by default, so the reference captures stay
    /// put.
    #[arg(long, default_value_t = 0)]
    movers: u32,
    /// Draw the movers with the camera's motion vectors alone, not their own (#79's A/B: TAA
    /// then smears them).
    #[arg(long)]
    no_mover_motion: bool,
    /// Leave the rivers' flow undisturbed by the movers (#107's A/B).
    #[arg(long)]
    no_floaters: bool,
    /// No waves from the movers in the lakes and the sea (#107's A/B for the wakes).
    #[arg(long)]
    no_wakes: bool,
    /// No spray where the water splashes: the steps' falls, a barrel dropped into a lake, the
    /// towed barrel's bow (#107's A/B for the splashes).
    #[arg(long)]
    no_splashes: bool,
    /// Draw the spray without the reactive mask, so TAA keeps its history there (the mask's
    /// A/B, #107).
    #[arg(long, hide = true)]
    no_reactive: bool,
    /// Holds the waves still at this many seconds (the shimmer's measure: what changes
    /// between frames of a still camera is then the aliasing alone).
    #[arg(long)]
    sea_time: Option<f64>,
    /// A settled probe updates every this many frames, on its turn (1 to 8; issue #103).
    #[arg(long, default_value_t = ProbeParams::default().cadence)]
    probe_cadence: u32,
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
    /// Reflect only the sky in the glass and the water, without the mirror rays against the
    /// scene (Y toggles them; devices without ray queries have none).
    #[arg(long)]
    no_ray_reflections: bool,
    /// Hard sun shadows: one ray to the sun's centre instead of its disc (Z toggles them).
    #[arg(long)]
    hard_shadows: bool,
    /// A day over the city: the sun rises in the east, crosses the south and sets in the west in
    /// this many seconds, then again, and the exposure follows it (issue #57).
    #[arg(long)]
    day: Option<f32>,
    /// The sun held where `--day` has it this far through the day (0 sunrise, 0.5 noon, 1
    /// sunset), the exposure metered from the scene as `--day`'s.
    #[arg(long, conflicts_with = "day")]
    time_of_day: Option<f32>,
    /// The island's golden shot of this name (`island --shot NAME`; the log lists them): its view
    /// and its time of day, unless `--view` or `--time-of-day` is given.
    #[arg(long)]
    shot: Option<String>,
    /// Fly the island's tour: up a steep valley to a lake, over the hills to the largest mouth
    /// and out to sea, resting at each shot (`island --tour`).
    #[arg(long, conflicts_with_all = ["fly", "orbit"])]
    tour: bool,
    /// Glide straight ahead from the start view at this many metres per second (a frame at a
    /// time with `--fixed-step`): a steady approach for measuring LOD pops (#131).
    #[arg(long, conflicts_with_all = ["fly", "orbit", "tour"])]
    dolly: Option<f32>,
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

impl Args {
    /// Whether the island draws its water: its sea, rivers and lakes (unless `--no-water`;
    /// `--water`, the old opt-in, asks for the default).
    fn water(&self) -> bool {
        self.legacy_water || !self.no_water
    }
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
    /// The island's sea (issue #105): the GPU's cascades and the CPU's spectra they came
    /// from (the start-up check), the seconds of waves played, and the time the last
    /// submitted frame's waves were at.
    water: Option<(WaterCascades, WaterSurface, Vec<Ocean>)>,
    sea_time: f64,
    sea_time_submitted: f32,
    /// `--movers` (#79): the barrels drifting down the island's rivers, on the sea's clock.
    barrels: Option<Barrels>,
    /// Their waves in the lakes and the sea (#107), with the water and the movers.
    wakes: Option<WaterWakes>,
    /// The spray where the water splashes (#107), with the water: the steps' falls, and with
    /// the movers the dropped barrel and the towed one's bow.
    splashes: Option<WaterSplashes>,
    falls: Vec<SplashSource>,
    /// The air the spray drifts in, m/s.
    wind: Vec3,
    /// The most drops alive at once, and those born, over the run.
    splash_peak: u32,
    splash_born: u64,
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
    /// `--tour`'s path and the seconds flown along it.
    tour: Option<island_demo::Tour>,
    tour_time: f32,
    /// The window's title, before the statistics it shows.
    title: &'static str,
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
    fn new(ctx: &mut Context, args: Args, cooked: Cooked, title: &'static str) -> Result<Self> {
        let args = island_demo::with_shot(args)?;
        let mut renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, ctx.extent())?;
        let mut taa = Taa::new(&ctx.device, &ctx.shaders, ctx.extent(), ctx.output.format)?;
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
        // Known before the scene, whose streamed pages it loads first (#121).
        let mut camera = start_camera(&args)?;
        let tour = if args.tour {
            Some(island_demo::Tour::new(&args)?)
        } else {
            None
        };
        if let Some(tour) = &tour {
            tour.place(&mut camera, 0.0);
            tracing::info!(seconds = %format_args!("{:.0}", tour.seconds()), "the island's tour");
        }
        if args.island.is_some() {
            let shots: Vec<String> = island_demo::island_shots(&args)
                .iter()
                .map(|s| {
                    let v = s.view();
                    format!(
                        "{} at {:.2}: {:.0},{:.1},{:.0},{:.1},{:.0}",
                        s.name, s.time, v[0], v[1], v[2], v[3], v[4]
                    )
                })
                .collect();
            tracing::info!(shots = %shots.join("  "), "the island's golden shots (--shot)");
        }
        let (scene, placed) = if args.gallery {
            build_gallery(ctx, &args, cooked)?
        } else if args.island.is_some() {
            (build_island(ctx, &args, cooked, &camera)?, Vec::new())
        } else {
            (build_city(ctx, &args, cooked, &camera)?, Vec::new())
        };
        let mut flags = CullFlags(CullFlags::CONE | CullFlags::FRUSTUM);
        // The city's views list 0.44–1.03 M clusters on their first frame, the island's 0.45 M:
        // reserved up front (16 MiB a frame slot, what growing reached anyway), no frame drops
        // any. Grown on demand, the first two frames missed the nearest buildings and rocks
        // (their fine clusters), a pop at the start whose trace TAA carried to frame 60.
        renderer.reserve_visible(1 << 21);
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
        anyhow::ensure!(
            (1..=8).contains(&args.probe_cadence),
            "--probe-cadence takes 1 to 8"
        );
        let probes = if ctx.device.features().ray_query && scene.rays().is_some() {
            let params = ProbeParams {
                rays: args.probe_rays,
                cascades: args.probe_cascades,
                cadence: args.probe_cadence,
                ..ProbeParams::default()
            };
            let probes = Probes::new(&ctx.device, &ctx.shaders, params)?;
            let p = probes.params();
            tracing::info!(
                probes = p.probe_count(),
                cascades = p.cascades,
                spacing_m = p.spacing,
                rays = p.rays,
                cadence = p.cadence,
                mib = %format_args!("{:.1}", probes.bytes() as f64 / f64::from(1 << 20)),
                "diffuse light probes"
            );
            Some(probes)
        } else {
            None
        };
        // The island's sea (issue #105): three cascades of FFT waves on the async compute
        // queue, their spectra from the CPU's. With it, the steps' falls that splash (#107) and
        // the air the spray drifts in: the sea's wind, slowed near the water.
        let mut falls = Vec::new();
        let mut wind = Vec3::ZERO;
        let water = if args.island.is_some() && args.water() {
            let seed = forge_core::Seed::new(args.island.unwrap_or(7)).derive(0x5EA);
            let oceans: Vec<Ocean> = OceanParams::cascades(seed).map(Ocean::new).into();
            let sea = &oceans[0].params;
            let (sin, cos) = (sea.wind_direction as f32).sin_cos();
            wind = SPRAY_WIND * sea.wind_speed as f32 * Vec3::new(cos, 0.0, sin);
            let descs: Vec<WaterCascadeDesc> = oceans
                .iter()
                .map(|o| {
                    let omega = o.mean_frequency();
                    WaterCascadeDesc {
                        patch: o.params.patch as f32,
                        choppiness: o.params.choppiness as f32,
                        samples: o.gpu_samples(),
                        omega: omega as f32,
                        shelf: forge_procgen::tma(omega, o.params.depth) as f32,
                    }
                })
                .collect();
            let water = WaterCascades::new(&ctx.device, &ctx.shaders, &descs)?;
            tracing::info!(
                cascades = water.len(),
                patches_m = ?oceans.iter().map(|o| o.params.patch).collect::<Vec<_>>(),
                significant_height_m = %format_args!(
                    "{:.2}",
                    oceans.iter().map(|o| o.significant_wave_height().powi(2)).sum::<f64>().sqrt()
                ),
                periods_s = %descs
                    .iter()
                    .map(|d| format!("{:.2}", std::f32::consts::TAU / d.omega))
                    .collect::<Vec<_>>()
                    .join(", "),
                mib = %format_args!("{:.1}", water.bytes() as f64 / f64::from(1 << 20)),
                "sea cascades"
            );
            // The shore the waves feel: the island's floor and its coast distance, and the
            // trains that come in to it, timed over the floor's profile.
            let height = island_heights(&args);
            let coast = forge_procgen::coast_distance(&height, 0.0, &TaskPool::client());
            let half = (0.5 * height.extent()) as f32;
            let profile = ShoreProfile::new(&height, &coast, 0.0, SHORE_BIN, SHORE_BINS);
            let tables: Vec<_> = SHORE_TRAINS.iter().map(|t| t.table(&profile)).collect();
            let trains: Vec<WaterShoreTrain> = SHORE_TRAINS
                .iter()
                .zip(&tables)
                .map(|(t, table)| WaterShoreTrain {
                    omega: t.omega() as f32,
                    height: t.height as f32,
                    table,
                })
                .collect();
            tracing::info!(
                depth_m = %[50.0, 100.0, 200.0, 500.0]
                    .map(|d| format!("{d:.0} m out {:.1}", profile.depth_at(d)))
                    .join(", "),
                seconds_to_shore_from_200_m = %tables
                    .iter()
                    .map(|t| format!("{:.0}", t[(200.0 / SHORE_BIN) as usize][0]))
                    .collect::<Vec<_>>()
                    .join(", "),
                "shore trains"
            );
            let IslandRivers {
                rivers,
                mouths,
                stones,
                lakes,
                falls: island_falls,
            } = island_ribbons(&height);
            falls = island_falls;
            let surface = WaterSurface::new(
                &ctx.device,
                &ctx.shaders,
                Some(WaterShore {
                    texels: height.size,
                    spacing: height.spacing as f32,
                    origin: [-half, -half],
                    floor: &height.data,
                    coast: &coast.data,
                    bin: SHORE_BIN as f32,
                    trains: &trains,
                    rivers: &rivers,
                    mouths: &mouths,
                    stones: &stones,
                    lakes: &lakes,
                }),
            )?;
            tracing::info!(
                mib = %format_args!("{:.1}", surface.river_bytes() as f64 / f64::from(1 << 20)),
                "the rivers' points and the ground they rest on"
            );
            Some((water, surface, oceans))
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
        // The movers (#79): barrels on the island's largest rivers.
        let barrels = (args.movers > 0 && args.island.is_some()).then(|| {
            let heights = island_heights(&args);
            let IslandRivers { rivers, lakes, .. } = island_ribbons(&heights);
            let barrels = Barrels::new(&rivers, &lakes, heights.spacing as f32, args.movers);
            // Views with the fixed step: of the first barrel at frame 60 (a second in) from 4 m to
            // its side and 1.5 m over it; of the first moored barrel the water runs past at
            // 1.2 m/s or more, at frame 60 from 6 m to its side and 7 m over it, looking a little
            // downstream; and of the towed barrel at frame 300, its wake grown, from 10 m inside
            // its circle and 8 m over it, looking back along its wake (#107).
            let view_of = |k: u32, time: f64, side_m: f32, up_m: f32, ahead_m: f32| {
                let (t, centre, ..) = barrels.pose(k, time);
                let side = (t.rotation * Vec3::Y).normalize();
                let down = Vec3::Y.cross(side).normalize_or_zero();
                let eye = centre + side_m * side + Vec3::new(0.0, up_m, 0.0);
                let look = (centre + ahead_m * down - eye).normalize();
                format!(
                    "{:.1},{:.2},{:.1},{:.1},{:.1}",
                    eye.x,
                    eye.y,
                    eye.z,
                    (-look.x).atan2(-look.z).to_degrees(),
                    look.y.asin().to_degrees()
                )
            };
            let view = view_of(0, 1.0, 4.0, 1.5, 0.0);
            let moored = (0..barrels.count)
                .find(|&k| barrels.moored(k) && barrels.place(k, 1.0).2 >= 1.2)
                .map_or(String::from("none"), |k| view_of(k, 1.0, 6.0, 7.0, 1.5));
            let towed = barrels.towed.map_or(String::from("none"), |_| {
                view_of(barrels.count - 1, 5.0, 10.0, 8.0, -4.0)
            });
            // The dropped barrel (#107's splashes) from 6 m off and 1.5 m over the water, looking
            // at where it meets it; with the fixed step it first does 2.73 s in, at frame 164.
            let dropped = barrels.dropped.map_or(String::from("none"), |(at, level)| {
                let eye = Vec3::new(at.x, level + 1.5, at.y + 6.0);
                let look = (Vec3::new(at.x, level + 0.6, at.y) - eye).normalize();
                format!(
                    "{:.1},{:.2},{:.1},0.0,{:.1}",
                    eye.x,
                    eye.y,
                    eye.z,
                    look.y.asin().to_degrees()
                )
            });
            tracing::info!(
                movers = args.movers,
                rivers = barrels.rivers.len(),
                %view,
                %moored,
                %towed,
                %dropped,
                tow_radius_m = barrels.towed.map_or(0.0, |t| t.1),
                "barrels on the rivers (--movers)"
            );
            barrels
        });
        let wakes = (barrels.is_some() && water.is_some() && !args.no_wakes)
            .then(|| WaterWakes::new(&ctx.device, &ctx.shaders))
            .transpose()?;
        let splashes = (water.is_some() && !args.no_splashes)
            .then(|| WaterSplashes::new(&ctx.device, &ctx.shaders))
            .transpose()?;
        if splashes.is_some() {
            tracing::info!(
                falls = falls.len(),
                wind = %format_args!("{:.1},{:.1},{:.1}", wind.x, wind.y, wind.z),
                capacity = forge_render::SPLASH_CAPACITY,
                "splashes"
            );
        }
        let mut gallery = Self {
            barrels,
            wakes,
            splashes,
            falls,
            wind,
            splash_peak: 0,
            splash_born: 0,
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
            water,
            sea_time: 0.0,
            sea_time_submitted: 0.0,
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
            tour,
            tour_time: 0.0,
            title,
            streaming: Vec::new(),
            frame_ms: Vec::new(),
            run_frame_ms: Vec::new(),
            last_frame: Instant::now(),
            started: None,
            settled: false,
        };
        if let Some(t) = gallery.args.time_of_day {
            gallery.set_sun_of_day(t.clamp(0.0, 1.0));
        }
        Ok(gallery)
    }

    /// Whether the exposure follows the scene's metered light (`--day`, `--time-of-day`) rather
    /// than `--ev100`.
    fn metered(&self) -> bool {
        self.args.day.is_some() || self.args.time_of_day.is_some()
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
        self.sea_time = match self.args.sea_time {
            Some(still) => still,
            None => self.sea_time + f64::from(self.step),
        };
        if let Some(length) = self.args.day {
            self.day_time += self.step;
            self.set_sun_of_day((self.day_time / length.max(1.0)).fract());
        }
        if let Some(tour) = &self.tour {
            self.tour_time += if self.args.fixed_step { 1.0 / 60.0 } else { dt };
            tour.place(&mut self.camera, self.tour_time);
        } else if self.args.fly {
            // Counter-clockwise seen from above, facing along the path, a little down.
            const RADIUS: f32 = 1400.0;
            const SPEED: f32 = 300.0;
            self.fly_time += if self.args.fixed_step { 1.0 / 60.0 } else { dt };
            let angle = self.fly_time * SPEED / RADIUS;
            self.camera.position = Vec3::new(angle.sin() * RADIUS, 140.0, angle.cos() * RADIUS);
            self.camera.yaw = angle - std::f32::consts::FRAC_PI_2;
            self.camera.pitch = -0.15;
        } else if let Some(speed) = self.args.dolly {
            let step = if self.args.fixed_step { 1.0 / 60.0 } else { dt };
            self.camera.position += self.camera.forward() * speed * step;
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
        // The target's format and the HDR settings (issue #94).
        let hdr = HdrOutput::new(ctx.output.peak, ctx.output.scene_stops, ctx.output.ui_white);
        self.taa.set_output(&ctx.shaders, ctx.output.format, hdr)?;
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
        if self.frame == 120
            && let Some((water, _, oceans)) = &self.water
        {
            water_check(&ctx.device, water, oceans, self.sea_time_submitted)?;
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
                // Probes still settling (age at most 8: new, or woken by a mover, #79).
                let young: Vec<usize> = states
                    .chunks(per)
                    .map(|c| c.iter().filter(|s| (s[3] / 4.0).floor() <= 8.0).count())
                    .collect();
                tracing::info!(
                    per_cascade = per,
                    ?active,
                    ?moved,
                    ?young,
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
        let metered = self.metered();
        let exposure = if metered {
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
        // The movers where they stand at the sea's time (#79).
        if let Some(barrels) = &self.barrels {
            self.scene.set_movers(&barrels.transforms(self.sea_time));
        }
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
        // The sea's waves (issue #105), on the async compute queue; the surface drawn from
        // them after the sky's compose.
        let waves = if let Some((water, _, _)) = &self.water {
            let time = self.sea_time as f32;
            self.sea_time_submitted = time;
            Some(water.update(&mut frame.graph, time))
        } else {
            None
        };
        // The sand the swash ran up is wet (#105) and the floor under the sea takes the waves'
        // caustics (#108): the resolve's layered ground reads the shore and the waves.
        let wet_ground = self.water.as_ref().and_then(|(cascades, surface, _)| {
            let caustics = waves
                .as_ref()
                .filter(|_| !self.args.no_caustics)
                .map(|waves| WaterCaustics {
                    cascades,
                    waves,
                    sun_dir: self.renderer.sun_dir,
                });
            surface.wet_ground(
                &mut frame.graph,
                frame.slot,
                self.sea_time_submitted,
                caustics,
            )
        });
        // The probes' light in place of the open sky's (issue #53): after the sky's tables,
        // which light their rays' misses, before the resolve.
        let probes = match &mut self.probes {
            Some(probes) if self.probes_on && self.sky_light => {
                if !self.probes_live {
                    probes.reset();
                }
                self.probes_live = true;
                // The rotations repeat with the jitter, once per round of turns (issue #103).
                let cycle = u64::from(self.taa.jitter_phases) * u64::from(probes.params().cadence);
                Some(probes.update(
                    &mut frame.graph,
                    frame.slot,
                    self.renderer.frame_address(frame.slot),
                    sky.light,
                    camera_in_scene,
                    self.taa.frame_index() % cycle,
                    targets.movers,
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
                wet_ground,
                movers: targets.movers,
            },
        );
        self.sky.compose(
            &mut frame.graph,
            &sky,
            targets.depth,
            taa_frame.color,
            extent,
        );
        // The splashes' reactive mask for TAA (#107), when spray is alive.
        let mut reactive = None;
        if let (Some((cascades, surface, _)), Some(waves)) = (&self.water, &waves) {
            // The barrels nearest the camera part the rivers' flow, and make waves in still
            // water (#107).
            let camera = Vec2::new(camera_in_scene.x, camera_in_scene.z);
            if let Some(barrels) = self.barrels.as_ref().filter(|_| !self.args.no_floaters) {
                surface.set_floaters(&barrels.floaters(self.sea_time, camera));
            }
            let wakes = self
                .wakes
                .as_ref()
                .zip(self.barrels.as_ref())
                .map(|(wakes, barrels)| {
                    wakes.update(
                        &mut frame.graph,
                        frame.slot,
                        &barrels.wakes(self.sea_time, camera),
                        camera_in_scene.as_dvec3(),
                        self.sea_time_submitted,
                    )
                });
            let requests = surface.draw(
                &mut frame.graph,
                frame.slot,
                cascades,
                waves,
                &sky,
                WaterSurfaceParams {
                    view_proj: taa_frame.jittered_projection * self.camera.view_rotation(),
                    camera: camera_in_scene.as_dvec3(),
                    sun_dir: self.renderer.sun_dir,
                    sun_radiance: self.renderer.sun_color
                        * (self.renderer.sun_illuminance * exposure),
                    sky_scale: self.renderer.sun_illuminance * exposure,
                    time: self.sea_time_submitted,
                    pixel: 2.0 / (taa_frame.jittered_projection.y_axis.y * extent.height as f32),
                    wakes,
                },
                taa_frame.color,
                targets.depth,
                extent,
            );
            // The island in the water and its shadow on it (#105): the water's mirror rays,
            // traced as the glass's (F, Y), and its shadow rays (J).
            let mirror_rays = self.flags.has(CullFlags::SKY_REFLECTIONS)
                && self.flags.has(CullFlags::RAY_REFLECTIONS);
            if mirror_rays || self.flags.has(CullFlags::SHADOWS) {
                self.renderer.trace_requested(
                    &mut frame.graph,
                    "water/reflections",
                    frame.slot,
                    requests,
                    taa_frame.color,
                    extent,
                    AmbientLight {
                        sky: self.sky_light.then_some(sky.light),
                        occlusion: None,
                        probes,
                        wet_ground: None,
                        movers: targets.movers,
                    },
                );
            }
            // The spray where it splashes (#107), over the water and its reflections.
            if let Some(splashes) = &self.splashes {
                let mut sources = self.falls.clone();
                if let Some(barrels) = &self.barrels {
                    barrels.splashes(self.sea_time, &mut sources);
                }
                let projection = taa_frame.jittered_projection;
                reactive = splashes.update(
                    &mut frame.graph,
                    frame.slot,
                    &sources,
                    &sky,
                    SplashParams {
                        view_proj: projection * self.camera.view_rotation(),
                        camera: camera_in_scene,
                        near: projection.w_axis.z,
                        focal: 0.5 * projection.y_axis.y * extent.height as f32,
                        sun_dir: self.renderer.sun_dir,
                        sun_radiance: self.renderer.sun_color
                            * (self.renderer.sun_illuminance * exposure),
                        sky_scale: self.renderer.sun_illuminance * exposure,
                        wind: self.wind,
                        time: self.sea_time_submitted,
                        shutter: 0.5 * self.step,
                        tlas: self.scene.rays().map_or(0, |r| r.tlas_address()),
                    },
                    taa_frame.color,
                    targets.depth,
                    extent,
                );
                let stats = splashes.stats();
                self.splash_peak = self.splash_peak.max(stats.live);
                self.splash_born += u64::from(stats.fresh);
            }
        }
        if metered {
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
        // The movers' own motion over the camera's (#79).
        if !self.args.no_mover_motion {
            self.renderer.mover_motion(
                &mut frame.graph,
                frame.slot,
                &self.scene,
                &targets,
                motion,
                camera.view_proj,
                taa_frame.previous_from_current,
                taa_frame.jitter,
            );
        }
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
            reactive.filter(|_| !self.args.no_reactive),
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
            "{} | {} instances, {:.1} M triangles, {:.1} M clusters | {}: drawn {:.0} k instances ({:.0} k hidden; cells {:.1} k listed, {:.1} k hidden whole, {:.2} k opened again), {:.0} k + {:.0} k clusters ({:.0} k in software, {:.1} k left to pass 2; {:.0} k work items, {:.0} k roots), {:.2} M tris | GPU {:.2} ms, frame p50 {p50:.2} p99 {p99:.2} ms",
            self.title,
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
        if let Some(splashes) = &self.splashes {
            tracing::info!(
                peak = self.splash_peak,
                born = self.splash_born,
                dropped = splashes.stats().dropped,
                "splashes' drops over the run (live at most, born, no room)"
            );
        }
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
            ("barrel", metal),
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
    /// hills: a deeper green, sand on the beaches, wet sand under the sea, and on the steep ground
    /// granite in the hills and limestone on the low land (D-042, #129; one dark rock with
    /// `--no-rock-types`). Then the sea's row, calm water, smooth and dark, for the plane that
    /// stands in for the sea until the water is drawn (D-038, `sea_prop`).
    fn island_ground(
        &mut self,
        layers: &[u8],
        texels: u32,
        size: f32,
        rock_types: bool,
        rock_sites: bool,
    ) -> Result<MaterialId> {
        let map = self
            .textures
            .add_layer_map("island layers", texels, texels, layers)?;
        let [rock, concrete, _, grass] = self.sets;
        // The valleys' own sets (#118), the island's only: generated here, in parallel.
        let start = Instant::now();
        let mut valley_sets: [Option<[TextureData; 2]>; 7] = Default::default();
        TaskPool::client().scope(|s| {
            for (i, slot) in valley_sets.iter_mut().enumerate() {
                s.spawn(move |_| {
                    *slot = Some(match i {
                        0 => textures::gravel(15, 512),
                        1 => textures::scree(16, 512),
                        2 => textures::scrub(17, 512),
                        3 => textures::shingle(18, 512),
                        4 => textures::granite(19, 512),
                        5 => textures::limestone(20, 512),
                        _ => textures::karst(21, 512),
                    });
                });
            }
        });
        let mut ids = Vec::new();
        for set in valley_sets.iter().flatten() {
            ids.push((self.textures.add(&set[0])?, self.textures.add(&set[1])?));
        }
        let [gravel, scree, scrub, shingle, granite, limestone, karst] =
            [ids[0], ids[1], ids[2], ids[3], ids[4], ids[5], ids[6]];
        tracing::info!(
            ms = start.elapsed().as_millis(),
            "island textures: gravel, scree, scrub, shingle, granite, limestone and karst"
        );
        let ground = self.table.add(Material::new(
            "island ground",
            RenderLayer {
                class: ShadingClass::Layered,
                albedo_texture: Some(map),
                texture_scale: size,
                // The layers' edges wander by a texel (4 m) instead of stepping along the map's
                // grid (#106).
                cavity: 1.0,
                // The sand's top follows the drawn ground's height, not the map's texels: they
                // drew it in teeth along the coast (#106).
                contour: Some(
                    LayerContour::new(
                        island_layer::SAND,
                        &[
                            island_layer::GRASS,
                            island_layer::DRY_GRASS,
                            island_layer::LUSH_GRASS,
                            island_layer::RIVERBANK,
                        ],
                        SAND_BELOW,
                        SAND_WANDER,
                    )
                    // The other beaches' tops follow it too (#128).
                    .with_others(&[island_layer::SHINGLE]),
                ),
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
                if rock_types {
                    // The hills' granite (D-042, #129): grey to pink, specked, smooth slabs.
                    textured(
                        granite,
                        [0.52, 0.48, 0.44],
                        [0.6, 0.55, 0.5],
                        7.0,
                        16.0,
                        0.05,
                    )
                } else {
                    textured(rock, [0.3, 0.3, 0.29], [0.38, 0.37, 0.35], 6.0, 14.0, 0.05)
                },
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
            (
                "island: dry grass",
                textured(grass, [0.9, 0.88, 0.55], [0.98, 0.94, 0.6], 12.0, 6.0, 0.02),
            ),
            (
                "island: lush grass",
                textured(grass, [0.55, 0.8, 0.45], [0.62, 0.88, 0.5], 12.0, 6.0, 0.02),
            ),
            (
                // The rivers' banks (D-041's riparian strip): reeds, sedges and shrubs, a deeper
                // and bluer green than the lush grass, the grass's texture at a coarser scale.
                "island: riverbanks",
                textured(grass, [0.36, 0.55, 0.3], [0.42, 0.63, 0.33], 7.0, 6.0, 0.02),
            ),
            (
                // Dark wet mud under the lakes: fine silt and what the plants left.
                "island: lake bed",
                textured(
                    concrete,
                    [0.2, 0.18, 0.14],
                    [0.26, 0.23, 0.17],
                    2.0,
                    10.0,
                    0.04,
                ),
            ),
            (
                // Cobbles and gravel in the steeper rivers' beds, a little glossy, a shade lighter than the
                // island's dark rock (#118).
                "island: gravel",
                textured(
                    gravel,
                    [0.45, 0.45, 0.44],
                    [0.5, 0.48, 0.45],
                    2.5,
                    18.0,
                    0.06,
                ),
            ),
            (
                // Scree at the foot of the valleys' walls: the broken rock, paler than the walls' (#118).
                "island: scree",
                textured(scree, [0.4, 0.4, 0.39], [0.46, 0.45, 0.43], 4.0, 14.0, 0.05),
            ),
            (
                // Scrub on the steep ground: the shrubs' clumps over stony soil (#118).
                "island: scrub",
                textured(scrub, [1.0, 1.0, 1.0], [1.1, 1.08, 0.95], 16.0, 6.0, 0.02),
            ),
            (
                // Silty sand on the deltas' fans under the lakes' shallows (#120): greyer and
                // darker than the beaches', lighter than the lakes' mud.
                "island: lake sand",
                textured(
                    concrete,
                    [0.46, 0.43, 0.34],
                    [0.52, 0.48, 0.38],
                    2.0,
                    10.0,
                    0.04,
                ),
            ),
            (
                // Shingle on the headlands' beaches (#128): rounded grey, blue-grey and brown
                // pebbles on coarse sand, that glint when wet.
                "island: shingle",
                textured(
                    shingle,
                    [0.66, 0.68, 0.7],
                    [0.74, 0.76, 0.78],
                    1.0,
                    20.0,
                    0.07,
                ),
            ),
            (
                // Limestone on the low ground's steep faces and the sea cliffs (D-042, #129):
                // pale cream-grey, pitted.
                "island: limestone",
                textured(
                    limestone,
                    [0.58, 0.58, 0.57],
                    [0.66, 0.65, 0.62],
                    6.0,
                    10.0,
                    0.04,
                ),
            ),
            (
                // Karst pavement on the limestone's driest gentler ground (#129): pale blocks a
                // metre or two across, moss in the fissures between them.
                "island: karst",
                textured(
                    karst,
                    [0.36, 0.36, 0.35],
                    [0.42, 0.42, 0.4],
                    9.0,
                    10.0,
                    0.04,
                ),
            ),
        ];
        assert_eq!(rows.len(), usize::from(island_layer::COUNT));
        for (name, layer) in rows {
            // Hex tiling (#66) on every textured row: grass, sand and rock are stochastic, and
            // the rock's 6 m repeat showed as a grid on the island's steep slopes.
            let hex_tiling = layer.albedo_texture.is_some();
            self.table.add(Material::new(
                name,
                RenderLayer {
                    hex_tiling,
                    ..layer
                },
            ));
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
        // The island's own stones (#130), in its granite and its limestone, and the rivers'
        // boulders in the granite they were carried down from.
        if rock_sites {
            let stones = |name, rock, a, b| {
                Material::new(
                    name,
                    RenderLayer {
                        cavity: 0.2,
                        ..textured(rock, a, b, 3.0, 16.0, 0.05)
                    },
                )
            };
            let granite = self.table.add(stones(
                "island: granite stones",
                granite,
                [0.52, 0.48, 0.44],
                [0.6, 0.55, 0.5],
            ));
            let limestone = self.table.add(stones(
                "island: limestone stones",
                limestone,
                [0.5, 0.5, 0.48],
                [0.58, 0.57, 0.54],
            ));
            for (i, (name, _, _)) in ISLAND_STONES.iter().enumerate() {
                let rock = if i < GRANITE_STONES {
                    granite
                } else {
                    limestone
                };
                self.by_prop.insert(name, rock);
            }
            for (i, (name, _, _)) in ISLAND_COBBLES.iter().enumerate() {
                let rock = if i < GRANITE_COBBLES {
                    granite
                } else {
                    limestone
                };
                self.by_prop.insert(name, rock);
            }
        }
        Ok(ground)
    }

    /// The row `prop` is made of (the default grey for a prop the table does not know). A part
    /// of a prop, `name@part` (the island's tiles), is made of the prop's.
    fn of(&self, prop: &str) -> MaterialId {
        let prop = prop.split_once('@').map_or(prop, |(whole, _)| whole);
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
    /// Grass on the driest ground: the ridges (`forge_procgen::paint_moisture`).
    pub const DRY_GRASS: u8 = 5;
    /// Grass on the wettest ground: the valley bottoms.
    pub const LUSH_GRASS: u8 = 6;
    /// The rivers' banks: reeds, sedges and shrubs on the grass within a few of a river's
    /// widths (D-041's riparian strip, `forge_procgen::paint_banks`). Its id was the rivers'
    /// bed's, which the water draws per pixel since #114.
    pub const RIVERBANK: u8 = 7;
    /// A lake's bed of dark mud, under a metre or more of its water (unless `--no-water`,
    /// `forge_procgen::paint_lake_beds`).
    pub const LAKEBED: u8 = 8;
    /// Cobbles and gravel: the beds and floors of the steeper rivers' reaches, under their water
    /// and beside it (#118, `forge_procgen::paint_valley_ground`).
    pub const GRAVEL: u8 = 9;
    /// Scree: broken rock at the foot of the steep walls of those rivers' valleys (#118).
    pub const SCREE: u8 = 10;
    /// Scrub: low shrubs on the steep ground that holds soil, the wetter rock (#118,
    /// `forge_procgen::paint_scrub`).
    pub const SCRUB: u8 = 11;
    /// Pale silty sand on the tops of the rivers' deltas' fans, under the lakes' shallow water in
    /// front of their mouths (#120, `forge_procgen::paint_fans`). Not the beaches' sand, whose
    /// top follows the coast's contour.
    pub const LAKE_SAND: u8 = 12;
    /// Shingle: the pebbles of the beaches on the headlands and under steep land (#128,
    /// `forge_procgen::paint_beaches`).
    pub const SHINGLE: u8 = 13;
    /// Limestone: the rock of the low ground and the sea cliffs, the old reefs raised with the
    /// island (D-042, #129, `forge_procgen::paint_geology`). The rock over it, `ROCK`, is the
    /// hills' granite.
    pub const LIMESTONE: u8 = 14;
    /// Karst: the limestone's bare pavements of blocks and fissures, on its driest gentler
    /// ground (#129).
    pub const KARST: u8 = 15;
    /// How many layers there are.
    pub const COUNT: u8 = 16;
}

/// The island's generation settings from the arguments (`--island`, `--island-spacing`,
/// `--island-steps`): the 16 km island of `forge-procgen` with its default erosion.
fn island_settings(args: &Args) -> (IslandParams, ErosionParams) {
    let seed = forge_core::Seed::new(args.island.unwrap_or(7));
    let mut params = IslandParams::island_16km(seed, args.island_spacing);
    params.plain = args.island_plain.unwrap_or(params.plain);
    params.plain_uplift = args.island_plain_uplift.unwrap_or(params.plain_uplift);
    params.plain_wander = args.island_plain_wander.unwrap_or(params.plain_wander);
    params.basins = args.island_basins.unwrap_or(params.basins);
    params.grade = args.island_grade.unwrap_or(params.grade);
    if let Some(from) = &args.island_wind {
        params.wind = forge_procgen::Wind::from_compass(from, args.island_rain_contrast);
        if params.wind.is_none() {
            tracing::warn!(wind = %from, "unknown wind origin (n, ne, e, se, s, sw, w, nw): the rain stays flat");
        }
    }
    let erosion = ErosionParams {
        steps: args.island_steps,
        channel_area: args.island_channel_ha * 10_000.0,
        ..ErosionParams::island()
    };
    (params, erosion)
}

/// How the island's rivers' valleys are carved (`--no-valleys`: not at all).
fn valley_params(args: &Args) -> Option<forge_procgen::ValleyParams> {
    (!args.no_valleys).then(forge_procgen::ValleyParams::default)
}

/// The island's heightfield, generated once and kept in `mesh-cache/` beside the cooked
/// meshes (`forge_procgen::cached_island`), then shaped for the sea and the rivers; made once a
/// process (the sea, the camera, the layers and the cook all ask for it).
fn island_heights(args: &Args) -> Field2<f32> {
    static MADE: std::sync::Mutex<Option<(String, Field2<f32>)>> = std::sync::Mutex::new(None);
    let (params, erosion) = island_settings(args);
    let key = format!(
        "{} {:?} {:?}",
        forge_procgen::island::island_key(&params, &erosion),
        valley_params(args),
        ribbon_params()
    );
    let mut made = MADE.lock().expect("the island's heights");
    if let Some((made_for, height)) = made.as_ref()
        && *made_for == key
    {
        return height.clone();
    }
    let height = make_island_heights(args);
    *made = Some((key, height.clone()));
    height
}

/// [`island_heights`], made.
fn make_island_heights(args: &Args) -> Field2<f32> {
    let (params, erosion) = island_settings(args);
    let dir = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR")).join("mesh-cache");
    let start = Instant::now();
    let pool = TaskPool::client();
    let (height, from_cache) = forge_procgen::cached_island(&dir, &params, &erosion, &pool)
        .expect("the island's cache file");
    // The sea floor under the flat sea (#96): the sea's plane then meets the ground along the
    // coast, between the samples.
    let mut height = height;
    forge_procgen::smooth_shore(&mut height, 0.0, f32::INFINITY, GROUND_SMOOTHING);
    let coast = forge_procgen::coast_distance(&height, 0.0, &pool);
    forge_procgen::sea_floor(&mut height, &coast, 0.0, SEA_FLOOR.0, SEA_FLOOR.1);
    // The ground within a few metres of the sea's level smoothed, so the coast runs smooth
    // instead of stepping with the samples (#106).
    forge_procgen::smooth_shore(&mut height, 0.0, SHORE_SMOOTHING.0, SHORE_SMOOTHING.1);
    // The rivers' valleys (#116, D-041): a floor for each river's water and, on its gentler
    // reaches, a bench or a floodplain, from the rivers traced over the field so far. The
    // rivers are traced again over the carved field (`island_water`).
    if let Some(valleys) = valley_params(args) {
        let carve = Instant::now();
        let flow = forge_procgen::drain(&height, 0.0, &pool);
        let rivers = island_rivers(&height, &flow);
        let (lakes, waters) = island_lake_waters(&height, &flow);
        // Without their steps (#122): the floors follow the rivers' fall, not their pools, so
        // the 8 m field is the same with them or without.
        let stepless = forge_procgen::RibbonParams {
            steps: None,
            ..ribbon_params()
        };
        let ribbons = forge_procgen::ribbons(&height, &rivers, &waters, &stepless);
        let s = forge_procgen::carve_valleys(&mut height, &ribbons, &lakes, &valleys, &pool);
        tracing::info!(
            points = %format_args!("{} floodplain, {} bench, {} room, {} in lakes", s.floodplain, s.bench, s.room, s.in_lake),
            floor_m = %format_args!("{:.1} of {:.1} asked", s.mean_floor.0, s.mean_floor.1),
            lowered = s.lowered,
            deepest_cut_m = %format_args!("{:.1}", s.deepest_cut),
            guarded = s.guarded,
            ms = carve.elapsed().as_millis(),
            "the rivers' valleys carved"
        );
    }
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

/// The island's rivers (stage 4 on the drawn field, as `genesis` traces them): where more than
/// 0.5 km² drains through a sample of `flow`.
fn island_rivers(height: &Field2<f32>, flow: &forge_procgen::Flow) -> forge_procgen::Rivers {
    let min_area = (500_000.0 / (height.spacing * height.spacing)) as u32 + 1;
    forge_procgen::trace_rivers(height, flow, min_area)
}

/// The island's lakes of a hectare or more, as `genesis` traces them: the priority flood's
/// water standing over half a metre above the drawn field.
fn island_lakes(height: &Field2<f32>, flow: &forge_procgen::Flow) -> forge_procgen::Lakes {
    let filled = forge_procgen::priority_flood(height, 0.0);
    forge_procgen::trace_lakes(height, &filled, flow, 0.5)
}

/// [`island_lakes`] and their water: the lakes of the rivers' `lake_area` or more as level
/// planes over the samples they stand over, which the rivers run into.
fn island_lake_waters(
    height: &Field2<f32>,
    flow: &forge_procgen::Flow,
) -> (forge_procgen::Lakes, Vec<forge_procgen::LakeWater>) {
    let filled = forge_procgen::priority_flood(height, 0.0);
    let lakes = forge_procgen::trace_lakes(height, &filled, flow, 0.5);
    let waters = forge_procgen::lake_waters(height, &filled, &lakes, ribbon_params().lake_area);
    (lakes, waters)
}

/// Whether the lakes' outlets rise into sills over their shallow arms, set once at start
/// (`--no-sills`, #120).
static SILLS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// The island's rivers' parameters, set once at start from the arguments (`--river-k`).
static RIBBON_PARAMS: std::sync::OnceLock<forge_procgen::RibbonParams> = std::sync::OnceLock::new();

/// The island's rivers' parameters: the island's, with `--river-k`'s size.
fn ribbon_params() -> forge_procgen::RibbonParams {
    RIBBON_PARAMS
        .get()
        .copied()
        .unwrap_or_else(forge_procgen::RibbonParams::island)
}

/// The island's water on the land (#105, D-038's rivers and lakes).
#[derive(Clone)]
struct IslandWater {
    /// Each river smoothed into a ribbon of points 4 m apart with its width, depth, speed and
    /// level, the tributaries first.
    ribbons: Vec<forge_procgen::Ribbon>,
    /// The channels carved for them, which the island's mesh draws (and the ribbons rest on far
    /// away).
    channels: forge_procgen::Channels,
    /// The lakes of a hectare or more, a level plane each over the samples it covers.
    lakes: Vec<forge_procgen::LakeWater>,
}

/// The island's rivers and lakes as water and beds, made once a process for the field it was
/// made from: the ground's layers and the water both ask for it at start (0.75 s each).
fn island_water(height: &Field2<f32>) -> IslandWater {
    static MADE: std::sync::Mutex<Option<(u64, IslandWater)>> = std::sync::Mutex::new(None);
    let key = height.digest() ^ height.spacing.to_bits() ^ u64::from(height.size);
    let mut made = MADE.lock().expect("the island's water");
    if let Some((made_for, water)) = made.as_ref()
        && *made_for == key
    {
        return water.clone();
    }
    let water = make_island_water(height);
    *made = Some((key, water.clone()));
    water
}

/// [`island_water`], made.
fn make_island_water(height: &Field2<f32>) -> IslandWater {
    let flow = forge_procgen::drain(height, 0.0, &TaskPool::client());
    let rivers = island_rivers(height, &flow);
    let (_, mut lakes) = island_lake_waters(height, &flow);
    let mut ribbons = forge_procgen::ribbons(height, &rivers, &lakes, &ribbon_params());
    let sills = SILLS.get().copied().unwrap_or(true);
    let channels = forge_procgen::Channels::new(
        height,
        &ribbons,
        &lakes,
        &forge_procgen::ChannelParams {
            sill: forge_procgen::ChannelParams::default()
                .sill
                .filter(|_| sills),
            ..forge_procgen::ChannelParams::default()
        },
    );
    // The lakes' water off the shallow arms past their outlets, which rise into sills (#120).
    if sills {
        let trimmed = forge_procgen::trim_outlets(&mut lakes, height, &ribbons);
        tracing::info!(trimmed, "the lakes' outlets' arms trimmed (samples)");
    }
    forge_procgen::rest_on(
        &mut ribbons,
        &|x, y| channels.height_at(height, x, y),
        &TaskPool::client(),
    );
    IslandWater {
        ribbons,
        channels,
        lakes,
    }
}

/// The seed of the stones in the island's rivers.
const RIVER_STONES: u64 = 0x5705_e105;

/// The seed of the rubble on the island's scree (#118).
const SCREE_RUBBLE: u64 = 0x5c2e_e2ab;

/// The stones in the island's rivers (#105): boulders on the carved beds, which the water flows
/// around.
fn island_stones(
    height: &Field2<f32>,
    ribbons: &[forge_procgen::Ribbon],
    channels: &forge_procgen::Channels,
) -> Vec<forge_procgen::Stone> {
    forge_procgen::stones(ribbons, channels, height, RIVER_STONES)
}

/// The island's rivers as the water draws them ([`island_ribbons`]), in the sea's frame.
struct IslandRivers {
    /// The tributaries first.
    rivers: Vec<Vec<WaterRiverPoint>>,
    /// Where they meet the sea.
    mouths: Vec<WaterMouth>,
    stones: Vec<WaterStone>,
    lakes: Vec<WaterLake>,
    /// The steps' falls that splash (#107).
    falls: Vec<SplashSource>,
}

/// The island's rivers as the water draws them (#105, D-038's rivers), in the sea's frame (the
/// field centred on the origin), the tributaries first, where they meet the sea, and the steps'
/// falls that splash (#107).
fn island_ribbons(height: &Field2<f32>) -> IslandRivers {
    let start = Instant::now();
    let IslandWater {
        ribbons,
        channels,
        lakes,
    } = island_water(height);
    let half = (0.5 * height.extent()) as f32;
    // Each step's fall (#122), in a few pieces across: where its sheet meets the pool below, its
    // line bowed downstream as the water draws it (`lip_shift`) and the sheet thrown on by the
    // speed over the lip for the time it falls.
    const PIECES: usize = 4;
    let step_falls: Vec<SplashSource> = ribbons
        .iter()
        .flat_map(|r| r.steps.iter().map(move |s| (r, s)))
        .enumerate()
        .flat_map(|(i, (r, s))| {
            let (lip, foot) = (r.points[s.lip as usize], r.points[s.foot as usize]);
            let down = Vec2::from(foot.direction);
            let side = Vec2::new(-down.y, down.x);
            let thrown = lip.speed * (2.0 * s.drop as f32 / 9.81).sqrt();
            (0..PIECES).map(move |k| {
                // The piece's middle, -1 at the right bank (seen downstream) to 1 at the left.
                let u = foot.half_width * ((k as f32 + 0.5) / PIECES as f32 * 2.0 - 1.0);
                let bow = forge_procgen::lip_shift(lip.lip, f64::from(u / foot.reach.max(1e-3)));
                let at = Vec2::from(foot.position) + side * u + down * (bow as f32 + thrown);
                SplashSource::Fall {
                    foot: Vec3::new(at.x - half, foot.level, at.y - half),
                    downstream: down,
                    half_width: foot.half_width / PIECES as f32,
                    drop: s.drop as f32,
                    speed: lip.speed,
                    depth: lip.depth,
                    seed: ((i * PIECES + k) as u32).wrapping_mul(0x9e37_79b9) ^ 0x5eed_fa11,
                }
            })
        })
        .collect();
    let points = ribbons.iter().flat_map(|r| &r.points);
    let widest = points
        .clone()
        .fold(0.0_f32, |m, p| m.max(2.0 * p.half_width));
    let steepest = points.clone().fold(0.0_f32, |m, p| m.max(p.grade));
    let deepest = points.clone().fold(0.0_f32, |m, p| m.max(p.depth));
    let fast = points.clone().filter(|p| p.speed >= 2.5).count();
    // Where the largest river (the last drawn) meets the sea, for placing a view.
    let mouth = ribbons
        .last()
        .and_then(|r| r.points.last())
        .map_or([0.0; 2], |p| [p.position[0] - half, p.position[1] - half]);
    // Views down a few rivers, 20 m upstream of a point halfway down, 3 m over the water or the
    // ground there: the largest, and the tenth, the twentieth and the thirtieth from it.
    let views: Vec<String> = [1, 10, 20, 30]
        .iter()
        .filter_map(|&k| ribbons.len().checked_sub(k))
        .map(|r| {
            let points = &ribbons[r].points;
            // Among the points the river had before its steps (#122), which they don't move.
            let unstepped = points.iter().filter(|p| p.key != u32::MAX);
            let p = *unstepped
                .clone()
                .nth(unstepped.count() / 2)
                .expect("a point");
            let (dx, dz) = (p.direction[0], p.direction[1]);
            let at = [p.position[0] - 20.0 * dx, p.position[1] - 20.0 * dz];
            let ground = channels.height_at(height, f64::from(at[0]), f64::from(at[1])) as f32;
            let y = p.unstepped.max(ground) + 3.0;
            let yaw = (-dx).atan2(-dz).to_degrees();
            format!(
                "{:.0},{y:.1},{:.0},{yaw:.1},-10",
                at[0] - half,
                at[1] - half
            )
        })
        .collect();
    tracing::info!(views = %views.join("  "), "views down the rivers (--view)");
    // The largest confluence (the largest tributary, which ends over the sea's level and not in
    // a lake), from 25 m back up it and 8 m over its water; and the largest river's head.
    let view_from = |p: &forge_procgen::RibbonPoint, back: f32, up: f32| {
        let (dx, dz) = (p.direction[0], p.direction[1]);
        let at = [p.position[0] - back * dx, p.position[1] - back * dz];
        let ground = channels.height_at(height, f64::from(at[0]), f64::from(at[1])) as f32;
        let yaw = (-dx).atan2(-dz).to_degrees();
        format!(
            "{:.0},{:.1},{:.0},{yaw:.1},-20",
            at[0] - half,
            p.unstepped.max(ground) + up,
            at[1] - half
        )
    };
    let confluence = ribbons
        .iter()
        .rev()
        .find(|r| {
            let end = r.points[r.points.len() - 1];
            end.level > 0.05 && r.lake_runs.is_empty()
        })
        .map_or_else(String::new, |r| {
            view_from(&r.points[r.points.len().saturating_sub(4)], 15.0, 8.0)
        });
    let head = ribbons
        .last()
        .map_or_else(String::new, |r| view_from(&r.points[8], 25.0, 6.0));
    tracing::info!(%confluence, %head, "a confluence and a head (--view)");
    // Where the water hands over: the largest river running into a lake, and the largest river
    // running into the sea, from 30 m back up it and 4 m over its water.
    let into_lake = ribbons
        .iter()
        .rev()
        .find_map(|r| Some((r, r.lake_runs.first()?[0] as usize)))
        .map_or_else(String::new, |(r, k)| view_from(&r.points[k], 30.0, 4.0));
    let into_sea = ribbons
        .last()
        .and_then(|r| Some((r, forge_procgen::sea_mouth(&r.points)?)))
        .map_or_else(String::new, |(r, k)| view_from(&r.points[k], 30.0, 4.0));
    // And the gentlest mouth of a river 6 m wide or more (its water's fall over its last 100 m),
    // where no white water hides the handover to the sea and the plume (the largest river's
    // reaches it in a cascade).
    let gentle_sea = ribbons
        .iter()
        .filter_map(|r| {
            let k = forge_procgen::sea_mouth(&r.points)?;
            let last = &r.points[k.saturating_sub(25)..=k];
            let fall = last.iter().map(|p| p.slope).sum::<f32>() / last.len() as f32;
            (r.points[k].half_width >= 3.0).then_some((fall, r, k))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map_or_else(String::new, |(_, r, k)| view_from(&r.points[k], 30.0, 4.0));
    tracing::info!(%into_lake, %into_sea, %gentle_sea, "where the rivers hand over (--view)");
    // Under the sea (#108): out from the largest river's mouth along its course to where the
    // floor lies 8 m deep, 3 m under the sea's level, looking back to the shore across the floor
    // and up at the surface.
    let under_sea = ribbons
        .last()
        .and_then(|r| Some(r.points[forge_procgen::sea_mouth(&r.points)?]))
        .and_then(|p| {
            let (dx, dz) = (p.direction[0], p.direction[1]);
            let at = (0..250)
                .map(|i| {
                    let s = 4.0 * i as f32;
                    [p.position[0] + s * dx, p.position[1] + s * dz]
                })
                .find(|at| {
                    channels.height_at(height, f64::from(at[0]), f64::from(at[1])) <= -8.0
                })?;
            let yaw = dx.atan2(dz).to_degrees();
            let (x, z) = (at[0] - half, at[1] - half);
            Some(format!(
                "{x:.0},-3.0,{z:.0},{yaw:.1},-10  {x:.0},-3.0,{z:.0},{yaw:.1},40"
            ))
        })
        .unwrap_or_default();
    tracing::info!(%under_sea, "under the sea (--view)");
    // Up a steep river from 2 m over its water, 40 m downstream of the steepest point of a river
    // 5 m wide or more (#113: from low, the water far up a valley).
    let up_valley = ribbons
        .iter()
        .flat_map(|r| r.points.iter())
        .filter(|p| p.half_width >= 2.5 && p.fade > 0.99)
        .max_by(|a, b| a.grade.total_cmp(&b.grade))
        .map_or_else(String::new, |p| {
            let (dx, dz) = (p.direction[0], p.direction[1]);
            let at = [p.position[0] + 40.0 * dx, p.position[1] + 40.0 * dz];
            let ground = channels.height_at(height, f64::from(at[0]), f64::from(at[1])) as f32;
            let yaw = dx.atan2(dz).to_degrees();
            format!(
                "{:.0},{:.1},{:.0},{yaw:.1},4",
                at[0] - half,
                ground + 2.0,
                at[1] - half
            )
        });
    tracing::info!(%up_valley, "up a steep river from low (--view)");
    // How far the water stands under its banks: the channel's depth less the water's (as it
    // was before the steps, #122, whose pools stand lower by design).
    let freeboard = points
        .clone()
        .map(|p| p.bank - p.unstepped)
        .fold(0.0_f32, f32::max);
    let cut_over_2_m = points
        .clone()
        .filter(|p| p.key != u32::MAX && p.bank - p.unstepped > 2.0)
        .count();
    // How steeply each river reaches the sea (#109): its water's fall over its last 160 m before
    // its mouth, the widest first, and how many fall over 5 % and over 10 % (a rapid's share).
    let mut falls: Vec<(f32, f32)> = ribbons
        .iter()
        .filter_map(|r| {
            let k = forge_procgen::sea_mouth(&r.points)?;
            let (mut back, mut run) = (k, 0.0_f32);
            while back > 0 && run < 160.0 {
                let (a, b) = (r.points[back - 1].position, r.points[back].position);
                run += (a[0] - b[0]).hypot(a[1] - b[1]);
                back -= 1;
            }
            (run > 0.0).then_some((
                2.0 * r.points[k].half_width,
                (r.points[back].level - r.points[k].level) / run,
            ))
        })
        .collect();
    falls.sort_by(|a, b| b.0.total_cmp(&a.0));
    let fall_list = falls
        .iter()
        .map(|(w, f)| format!("{w:.0}m {:.0}%", 100.0 * f))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(
        mouths = falls.len(),
        over_5_pct = falls.iter().filter(|f| f.1 > 0.05).count(),
        over_10_pct = falls.iter().filter(|f| f.1 > 0.1).count(),
        width_and_fall = %fall_list,
        "the rivers' last 160 m to the sea"
    );
    // The rivers' reaches by D-041's types (their water's slope): steps and pools in a V valley
    // over 4 %, rapids on a bench from 2 to 4 %, a floodplain under 2 %; in the hills (over
    // 20 m) and on the plain.
    let reaches = |hills: bool| {
        let mut count = [0_usize; 3];
        for p in ribbons
            .iter()
            .flat_map(|r| &r.points)
            .filter(|p| p.fade > 0.5 && p.key != u32::MAX && (p.unstepped > 20.0) == hills)
        {
            count[usize::from(p.grade <= 0.04) + usize::from(p.grade < 0.02)] += 1;
        }
        format!(
            "{} V, {} bench, {} floodplain",
            count[0], count[1], count[2]
        )
    };
    tracing::info!(
        hills = %reaches(true),
        plain = %reaches(false),
        "the rivers' reaches (points over 4 %, 2-4 %, under 2 %)"
    );
    // The steep reaches' steps and pools (#122): how many, how far apart in the river's widths,
    // how high; and the highest on a river 5 m wide or more, from 15 m down its pool and 3 m
    // over its water, looking up at the fall.
    let steps: Vec<(&forge_procgen::Ribbon, &forge_procgen::Step)> = ribbons
        .iter()
        .flat_map(|r| r.steps.iter().map(move |s| (r, s)))
        .collect();
    if !steps.is_empty() {
        let count = steps.len() as f64;
        let widths = steps.iter().map(|(_, s)| s.spacing / s.width).sum::<f64>() / count;
        let drop = steps.iter().map(|(_, s)| s.drop).sum::<f64>() / count;
        let highest = steps.iter().map(|(_, s)| s.drop).fold(0.0, f64::max);
        let view = steps
            .iter()
            .filter(|(_, s)| s.width >= 5.0)
            .max_by(|a, b| a.1.drop.total_cmp(&b.1.drop))
            .map_or_else(String::new, |(r, s)| {
                let p = r.points[s.foot as usize];
                let (dx, dz) = (p.direction[0], p.direction[1]);
                let at = [p.position[0] + 15.0 * dx, p.position[1] + 15.0 * dz];
                let yaw = dx.atan2(dz).to_degrees();
                format!(
                    "{:.0},{:.1},{:.0},{yaw:.1},-5",
                    at[0] - half,
                    p.level + 3.0,
                    at[1] - half
                )
            });
        tracing::info!(
            steps = steps.len(),
            rivers = ribbons.iter().filter(|r| !r.steps.is_empty()).count(),
            spacing_widths = %format_args!("{widths:.2}"),
            drop_m = %format_args!("{drop:.2}"),
            highest_m = %format_args!("{highest:.2}"),
            %view,
            "the steep rivers' steps and pools (--view)"
        );
    }
    tracing::info!(
        rivers = ribbons.len(),
        largest_mouth = %format_args!("{:.0},{:.0}", mouth[0], mouth[1]),
        points = points.clone().count(),
        widest_m = %format_args!("{widest:.1}"),
        deepest_m = %format_args!("{deepest:.2}"),
        steepest = %format_args!("{steepest:.2}"),
        points_over_2_5_m_s = fast,
        most_under_its_banks_m = %format_args!("{freeboard:.1}"),
        points_over_2_m_under = cut_over_2_m,
        refined_cells = channels.refined().len(),
        ms = start.elapsed().as_millis(),
        "island rivers"
    );
    // The confluences' rounded corners (#119), and the four largest tributaries' junctions from
    // 40 m over their corners.
    let corners: Vec<&forge_procgen::Corner> = ribbons.iter().flat_map(|r| &r.corners).collect();
    let radii = corners.iter().map(|c| c.radius);
    let mean_radius = radii.clone().sum::<f64>() / corners.len().max(1) as f64;
    let views: Vec<String> = ribbons
        .iter()
        .rev()
        .filter(|r| !r.corners.is_empty())
        .take(4)
        .map(|r| {
            let n = r.corners.len() as f64;
            let mean = |i: usize| r.corners.iter().map(|c| c.tip[i]).sum::<f64>() / n;
            let level = r.corners.iter().map(|c| c.level[1]).sum::<f64>() / n;
            format!(
                "{:.0},{:.0},{:.0},0,-89",
                mean(0) - f64::from(half),
                level + 40.0,
                mean(1) - f64::from(half)
            )
        })
        .collect();
    tracing::info!(
        junctions = ribbons.iter().filter(|r| !r.corners.is_empty()).count(),
        corners = corners.len(),
        mean_radius_m = %format_args!("{mean_radius:.1}"),
        largest_radius_m = %format_args!("{:.1}", radii.fold(0.0, f64::max)),
        views = %views.join("  "),
        "the confluences' corners rounded (--view)"
    );
    // Where the rivers meet the sea, and where they run into a lake or out of one: from there the
    // sea's or the lake's water carries their flow on, or draws it in.
    let mouth = |p: &forge_procgen::RibbonPoint| WaterMouth {
        position: [p.position[0] - half, p.position[1] - half],
        direction: p.direction,
        half_width: p.half_width,
        speed: p.speed,
        white: 0.0,
    };
    // How much of a river runs white over its last 16 m before the sea: the rapids' share of
    // `river_frag_main` (`water.slang`), by its fall and its speed.
    let smoothstep = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let white = |points: &[forge_procgen::RibbonPoint], k: usize| {
        points[k.saturating_sub(4)..=k]
            .iter()
            .map(|p| smoothstep(0.06, 0.2, p.slope) * smoothstep(1.5, 3.0, p.speed))
            .fold(0.0, f32::max)
    };
    let mut mouths: Vec<WaterMouth> = ribbons
        .iter()
        .filter_map(|r| {
            let k = forge_procgen::sea_mouth(&r.points)?;
            Some(WaterMouth {
                white: white(&r.points, k),
                ..mouth(&r.points[k])
            })
        })
        .collect();
    let sea_mouths = mouths.len();
    for r in &ribbons {
        for &[k, last] in &r.lake_runs {
            // The speed the river comes in at: the point before the lake's.
            let before = r.points[(k as usize).saturating_sub(1)];
            mouths.push(WaterMouth {
                speed: before.speed,
                ..mouth(&r.points[k as usize])
            });
            // And where it runs out (#120): the lake's water drawn into the river, the river's
            // own in a cone back into the lake, so the two meet as one water past the lip.
            if let Some(out) = r.points.get(last as usize + 1) {
                mouths.push(WaterMouth {
                    direction: [-out.direction[0], -out.direction[1]],
                    speed: -out.speed,
                    ..mouth(out)
                });
            }
        }
    }
    let spacing = height.spacing as f32;
    // Views of the three largest lakes' outlets, from 25 m over the water 20 m south of them,
    // looking down at where the river leaves.
    let mut by_size: Vec<&forge_procgen::LakeWater> = lakes.iter().collect();
    by_size.sort_by_key(|l| std::cmp::Reverse(l.mask.iter().filter(|&&m| m).count()));
    let outlets: Vec<String> = by_size
        .iter()
        .take(3)
        .map(|l| {
            let (x, z) = (
                l.outlet[0] as f32 * spacing - half,
                l.outlet[1] as f32 * spacing - half,
            );
            format!("{x:.0},{:.1},{:.0},0,-45", l.level + 25.0, z + 20.0)
        })
        .collect();
    tracing::info!(views = %outlets.join("  "), "the largest lakes' outlets (--view)");
    // Where the two largest rivers through a lake run into it and out of it (#120): the first
    // point of a run in a lake, and the lip past it, where the water falls from the lake's
    // level; from 40 m up the river (into it) or down it (out of it), 12 m over the water,
    // looking at the junction.
    let junction = |p: &forge_procgen::RibbonPoint, downstream: bool| {
        let (dx, dz) = (p.direction[0], p.direction[1]);
        let back = if downstream { -40.0 } else { 40.0 };
        let at = [p.position[0] - back * dx, p.position[1] - back * dz];
        let yaw = if downstream {
            dx.atan2(dz)
        } else {
            (-dx).atan2(-dz)
        };
        format!(
            "{:.0},{:.1},{:.0},{:.1},-20",
            at[0] - half,
            p.level + 12.0,
            at[1] - half,
            yaw.to_degrees()
        )
    };
    let (mut into, mut out_of) = (Vec::new(), Vec::new());
    for r in ribbons.iter().rev() {
        for &[first, last] in &r.lake_runs {
            if first > 0 && into.len() < 2 {
                into.push(junction(&r.points[first as usize], false));
            }
            let lake = r.points[last as usize].level;
            if let Some(p) = r.points[last as usize..]
                .iter()
                .find(|p| p.level < lake - 0.05)
                && out_of.len() < 2
            {
                out_of.push(junction(p, true));
            }
        }
    }
    tracing::info!(
        into = %into.join("  "),
        out_of = %out_of.join("  "),
        "rivers into and out of the lakes (--view)"
    );
    // The rivers' deltas (#120): their fans' lengths, and the two longest from 40 m back up the
    // river, 12 m over the lake, looking down it at the fan, and from 70 m over the fan's middle.
    let mut deltas: Vec<&forge_procgen::Delta> =
        ribbons.iter().flat_map(|r| r.deltas.iter()).collect();
    deltas.sort_by(|a, b| b.length.total_cmp(&a.length));
    let half_m = f64::from(half);
    let delta_views: Vec<String> = deltas
        .iter()
        .take(2)
        .map(|d| {
            let back = [d.apex[0] - 40.0 * d.down[0], d.apex[1] - 40.0 * d.down[1]];
            let yaw = (-d.down[0]).atan2(-d.down[1]).to_degrees();
            let mid = [
                d.apex[0] + 0.5 * d.length * d.down[0],
                d.apex[1] + 0.5 * d.length * d.down[1],
            ];
            format!(
                "{:.0},{:.1},{:.0},{yaw:.1},-20  {:.0},{:.1},{:.0},0,-89",
                back[0] - half_m,
                d.level + 12.0,
                back[1] - half_m,
                mid[0] - half_m,
                d.level + 70.0,
                mid[1] - half_m,
            )
        })
        .collect();
    tracing::info!(
        deltas = deltas.len(),
        fans_m = %deltas.iter().map(|d| format!("{:.0}", d.length)).collect::<Vec<_>>().join(" "),
        views = %delta_views.join("  "),
        "the rivers' deltas into the lakes (--view)"
    );
    // The bars in the large mouths at the sea (#127): per mouth with bars, how many and the
    // longest's length; and the two largest such mouths from 40 m back up the river from the
    // bars' upstream tip, 15 m over the water, and from straight over the bars' middle.
    let barred: Vec<&forge_procgen::Ribbon> = ribbons
        .iter()
        .rev()
        .filter(|r| !r.bars.is_empty())
        .collect();
    let bar_views: Vec<String> = barred
        .iter()
        .take(2)
        .map(|r| {
            let (b, n) = (r.bars[0], r.bars.len() as f64);
            let mid = r.bars.iter().fold([0.0; 2], |m, b| {
                [m[0] + b.centre[0] / n, m[1] + b.centre[1] / n]
            });
            let length = r.bars.iter().fold(0.0_f64, |m, b| m.max(2.0 * b.half[0]));
            let back = 0.5 * length + 40.0;
            let at = [mid[0] - back * b.down[0], mid[1] - back * b.down[1]];
            let yaw = (-b.down[0]).atan2(-b.down[1]).to_degrees();
            format!(
                "{:.0},{:.1},{:.0},{yaw:.1},-20  {:.0},{:.1},{:.0},0,-89",
                at[0] - half_m,
                b.level[0] + 15.0,
                at[1] - half_m,
                mid[0] - half_m,
                b.level[1] + 1.2 * length,
                mid[1] - half_m,
            )
        })
        .collect();
    tracing::info!(
        mouths = barred.len(),
        bars = %barred
            .iter()
            .map(|r| {
                let longest = r.bars.iter().fold(0.0_f64, |m, b| m.max(2.0 * b.half[0]));
                format!("{}x{longest:.0}m", r.bars.len())
            })
            .collect::<Vec<_>>()
            .join(" "),
        views = %bar_views.join("  "),
        "the bars in the large mouths (--view)"
    );
    let lakes: Vec<WaterLake> = lakes
        .iter()
        .map(|l| WaterLake {
            level: l.level,
            depth: l.depth,
            origin: [
                l.first[0] as f32 * spacing - half,
                l.first[1] as f32 * spacing - half,
            ],
            size: l.size,
            mask: l.mask.clone(),
        })
        .collect();
    // Views of the three largest, from 30 m over the water past the south edge of the mask,
    // looking north across it.
    let mut largest: Vec<&WaterLake> = lakes.iter().collect();
    largest.sort_by_key(|l| std::cmp::Reverse(l.mask.iter().filter(|&&m| m).count()));
    let views: Vec<String> = largest
        .iter()
        .take(3)
        .map(|l| {
            let x = l.origin[0] + 0.5 * l.size[0] as f32 * spacing;
            let z = l.origin[1] + (l.size[1] as f32 + 4.0) * spacing;
            format!("{x:.0},{:.1},{z:.0},0,-15", l.level + 30.0)
        })
        .collect();
    // And under the largest (#108): at its deepest sample, halfway down its depth, looking up
    // at its surface and across its floor.
    let under = largest
        .first()
        .and_then(|l| {
            let at = |k: usize| {
                let (i, j) = (k % l.size[0] as usize, k / l.size[0] as usize);
                [
                    l.origin[0] + i as f32 * spacing,
                    l.origin[1] + j as f32 * spacing,
                ]
            };
            let ground = |p: [f32; 2]| {
                channels.height_at(height, f64::from(p[0] + half), f64::from(p[1] + half)) as f32
            };
            let deepest = (0..l.mask.len())
                .filter(|&k| l.mask[k])
                .map(at)
                .min_by(|a, b| ground(*a).total_cmp(&ground(*b)))?;
            let y = 0.5 * (l.level + ground(deepest));
            Some(format!(
                "{:.0},{y:.1},{:.0},0,25  {:.0},{y:.1},{:.0},0,-20",
                deepest[0], deepest[1], deepest[0], deepest[1]
            ))
        })
        .unwrap_or_default();
    tracing::info!(
        lakes = lakes.len(),
        sea_mouths,
        lake_mouths = mouths.len() - sea_mouths,
        views = %views.join("  "),
        %under,
        "the island's lakes (--view)"
    );
    let rivers: Vec<Vec<WaterRiverPoint>> = ribbons
        .iter()
        .map(|r| {
            r.points
                .iter()
                .zip(forge_procgen::bar_spans(r))
                .map(|(p, bars)| WaterRiverPoint {
                    position: [p.position[0] - half, p.position[1] - half],
                    level: p.level,
                    direction: p.direction,
                    half_width: p.half_width,
                    cover: p.cover,
                    reach: p.reach,
                    depth: p.depth,
                    bank: p.bank,
                    speed: p.speed,
                    slope: p.slope,
                    foam: p.foam,
                    lip: p.lip,
                    fade: p.fade,
                    ground: p.ground,
                    bars,
                })
                .collect()
        })
        .collect();
    // The stones, by the index of their point among every river's points.
    let mut first = Vec::with_capacity(ribbons.len());
    let mut count = 0u32;
    for r in &ribbons {
        first.push(count);
        count += r.points.len() as u32;
    }
    let stones: Vec<WaterStone> = island_stones(height, &ribbons, &channels)
        .iter()
        .map(|s| WaterStone {
            position: [s.position[0] as f32 - half, s.position[1] as f32 - half],
            waterline: s.waterline() as f32,
            radius: s.radius as f32,
            point: first[s.ribbon as usize] + s.point,
        })
        .collect();
    // A view of the stone in the fastest water that breaks it: 10 m upstream, 2.5 m over the
    // water, looking down the river at it.
    let points: Vec<&WaterRiverPoint> = rivers.iter().flatten().collect();
    let stone_view = stones
        .iter()
        .filter(|s| s.waterline > 0.4)
        .max_by(|a, b| {
            points[a.point as usize]
                .speed
                .total_cmp(&points[b.point as usize].speed)
        })
        .map_or_else(String::new, |s| {
            let p = points[s.point as usize];
            let (dx, dz) = (p.direction[0], p.direction[1]);
            let yaw = (-dx).atan2(-dz).to_degrees();
            format!(
                "{:.0},{:.1},{:.0},{yaw:.1},-14",
                s.position[0] - 10.0 * dx,
                p.level + 2.5,
                s.position[1] - 10.0 * dz
            )
        });
    tracing::info!(
        stones = stones.len(),
        breaking_the_water = stones.iter().filter(|s| s.waterline > 0.0).count(),
        mouths = mouths.len(),
        stone_view = %stone_view,
        "the rivers' stones and mouths"
    );
    IslandRivers {
        rivers,
        mouths,
        stones,
        lakes,
        falls: step_falls,
    }
}

/// The island's sea floor (`forge_procgen::sea_floor`): metres of depth it levels off at, and
/// the metres from the coast that set its slope (60 over 1 500: 4 % at the shore).
const SEA_FLOOR: (f32, f32) = (60.0, 1500.0);

/// Passes of the binomial filter over the whole field (#106): the 8 m erosion leaves steps two
/// samples apart on the slopes, which one pass takes out.
const GROUND_SMOOTHING: u32 = 1;

/// The shore's smoothing (`forge_procgen::smooth_shore`, #106): the samples within 3.5 m of the
/// sea's level (the sand's top at 2.5 m among them), four passes of the binomial filter.
const SHORE_SMOOTHING: (f32, u32) = (3.5, 4);

/// Metres above the sea under which the island's gentle ground is sand (the layer map's rule,
/// and the ground's contour under each pixel).
const SAND_BELOW: f32 = 2.5;

/// The rivers' riparian strip (D-041, `island_layer::RIVERBANK`): metres past the water, plus
/// this many of the river's widths.
const RIPARIAN_STRIP: (f64, f64) = (6.0, 2.0);

/// Metres of a lake's water over the ground from which the map paints its bed of mud (#114):
/// the lookup's blend and wander carry a texel's layer up to 8 m, so a bed painted to the
/// water's edge showed on the shore; nearer the edge the water draws its bed itself.
const LAKEBED_UNDER: f64 = 1.0;

/// Metres the sand's top wanders up and down along the coast (`LayerContour::wander`, #106).
const SAND_WANDER: f32 = 0.3;

/// The shore's wave trains (#105): the breeze's swell as three periods around its peak (the
/// swell cascade centres on 7.3 s), heights in deep water.
const SHORE_TRAINS: [ShoreTrain; 3] = [
    ShoreTrain {
        period: 9.0,
        height: 0.9,
    },
    ShoreTrain {
        period: 7.0,
        height: 0.6,
    },
    ShoreTrain {
        period: 12.0,
        height: 0.5,
    },
];
/// Metres a bin of the trains' tables, and the bins: 4 km out.
const SHORE_BIN: f64 = 4.0;
const SHORE_BINS: usize = 1024;

/// Tiles a side of the island's ground (#106): 2 km each over the 16 km.
const ISLAND_TILES: u32 = 8;

/// The island's ground as props (on its layered ground, `CityMaterials::island_ground`), its
/// samples generated only when a tile's cooked mesh is not in the cache. In tiles (#106), each
/// cooked and cached on its own, their borders locked in every level so they meet without a
/// crack; named `island@x-z`, drawn finer `island@x-z-2m` (the cache keeps one file per name,
/// so the two stay side by side; the material is the name's before the `@`).
fn island_tiles(args: &Args) -> Vec<PropSpec> {
    let (params, erosion) = island_settings(args);
    let factor = island_factor(args);
    let size = (params.size - 1) * factor + 1;
    let spacing = params.spacing / f64::from(factor);
    let mut key = format!(
        "{}, smoothed {} passes, sea floor {} m over {} m, shore smoothed {:?}, rivers {:?} carved {:?} in valleys {:?}",
        forge_procgen::island::island_key(&params, &erosion),
        GROUND_SMOOTHING,
        SEA_FLOOR.0,
        SEA_FLOOR.1,
        SHORE_SMOOTHING,
        ribbon_params(),
        forge_procgen::ChannelParams::default(),
        valley_params(args),
    );
    if factor > 1 {
        key += &format!(", drawn on the cubic at {spacing} m");
        if args.island_detail > 0.0 {
            key += &format!(
                ", amplified {:?} x {} faded over {:?} m",
                forge_procgen::AmplifyParams::island(forge_core::Seed::new(0)),
                args.island_detail,
                DETAIL_FADE
            );
        }
    }
    let for_source = args.clone();
    let source: Arc<dyn Fn() -> Arc<[f32]> + Send + Sync> =
        Arc::new(move || island_drawn(&for_source).heights.clone());
    // The rivers' channels, carved into cells drawn in quads of a metre (#105).
    let for_detail = args.clone();
    let detail: Arc<forge_geom::city::DetailSource> =
        Arc::new(move |_: &[f32]| island_drawn(&for_detail).detail.clone());
    let cells = size - 1;
    let edges: Vec<u32> = (0..=ISLAND_TILES)
        .map(|t| t * cells / ISLAND_TILES)
        .collect();
    let mut tiles = Vec::new();
    for tz in 0..ISLAND_TILES as usize {
        for tx in 0..ISLAND_TILES as usize {
            tiles.push(PropSpec {
                name: if factor > 1 {
                    format!("island@{tx}-{tz}-{spacing}m")
                } else {
                    format!("island@{tx}-{tz}")
                },
                kind: PropKind::Heightfield(Heightfield {
                    key: key.clone(),
                    samples: size,
                    spacing: spacing as f32,
                    source: source.clone(),
                    detail: Some(detail.clone()),
                    window: Some(CellWindow {
                        first: [edges[tx], edges[tz]],
                        cells: [edges[tx + 1] - edges[tx], edges[tz + 1] - edges[tz]],
                    }),
                }),
            });
        }
    }
    tiles
}

/// How many times finer than the field the island's ground is drawn (`--island-drawn`, #106).
fn island_factor(args: &Args) -> u32 {
    let factor = (args.island_spacing / args.island_drawn).round().max(1.0) as u32;
    let split = forge_procgen::ChannelParams::default().split;
    assert!(
        split.is_multiple_of(factor),
        "--island-drawn divides the field's {} m spacing by 1, 2, 4 or 8",
        args.island_spacing
    );
    factor
}

/// Metres past the water's reach (the refined cells, the lakes, the ground under the shore's
/// 3.5 m) over which the amplification's detail fades in (#106): the channels, the lakes'
/// shores and the beaches keep the ground the water was made for.
const DETAIL_FADE: (f32, f32) = (4.0, 32.0);

/// The island's ground as its tiles draw it (#106): the samples, and the cells drawn finer
/// (the rivers' channels, the lakes' shores and the coast's contours in quads of a metre).
struct DrawnGround {
    /// Samples a side.
    size: u32,
    /// Metres between them.
    spacing: f64,
    heights: Arc<[f32]>,
    detail: Arc<HeightfieldDetail>,
}

impl DrawnGround {
    /// The ground at (x, y) metres in the field's frame as its coarse cells draw it: split along
    /// their (i + 1, j) – (i, j + 1) diagonal (`forge_procgen::drawn_height`).
    fn height_at(&self, x: f64, y: f64) -> f64 {
        let n = self.size as usize;
        let last = f64::from(self.size - 2);
        let (gx, gy) = (x / self.spacing, y / self.spacing);
        let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
        let (tx, ty) = ((gx - cx).clamp(0.0, 1.0), (gy - cy).clamp(0.0, 1.0));
        let at = |i: usize, j: usize| f64::from(self.heights[j * n + i]);
        let (i, j) = (cx as usize, cy as usize);
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        if tx + ty <= 1.0 {
            a + tx * (b - a) + ty * (c - a)
        } else {
            d + (1.0 - tx) * (c - d) + (1.0 - ty) * (b - d)
        }
    }
}

/// The island's field amplified `factor` times finer (stage 5): `forge_procgen::amplify` at
/// each halving of the spacing, over the drainage traced at that spacing.
fn island_amplified(height: &Field2<f32>, factor: u32, seed: u64, pool: &TaskPool) -> Field2<f32> {
    let mut field = height.clone();
    for level in 0..factor.trailing_zeros() {
        let flow = forge_procgen::drain(&field, 0.0, pool);
        let params = forge_procgen::AmplifyParams::island(forge_core::Seed::new(
            seed ^ (0xa3f1_0000 + u64::from(level)),
        ));
        field = forge_procgen::amplify(&field, &flow.area, 0.0, &params, pool);
    }
    field
}

/// [`DrawnGround`], made once a process for the field and the factor: every tile asks for it.
/// At the field's spacing, the field's samples and its refined cells on the cells' planes; finer,
/// the field's cubic with the channels carved, its samples and its refined cells
/// (`Channels::fine`), and away from the water the amplification's detail (`--island-detail`).
fn island_drawn(args: &Args) -> Arc<DrawnGround> {
    static MADE: std::sync::Mutex<Option<(u64, Arc<DrawnGround>)>> = std::sync::Mutex::new(None);
    let height = island_heights(args);
    let factor = island_factor(args);
    let key = height.digest()
        ^ height.spacing.to_bits()
        ^ u64::from(height.size)
        ^ u64::from(factor).rotate_left(48)
        ^ u64::from(args.island_detail.to_bits()).rotate_left(16);
    let mut made = MADE.lock().expect("the island's drawn ground");
    if let Some((made_for, drawn)) = made.as_ref()
        && *made_for == key
    {
        return drawn.clone();
    }
    let start = Instant::now();
    let IslandWater {
        channels, lakes, ..
    } = island_water(&height);
    let pool = TaskPool::client();
    let (size, spacing) = (height.size, height.spacing);
    let mut detail_ms = 0;
    let drawn = if factor == 1 {
        let heights = channels.detail(&height, &pool);
        DrawnGround {
            size,
            spacing,
            heights: Arc::from(height.data),
            detail: Arc::new(HeightfieldDetail {
                split: channels.params().split,
                cells: channels.refined().to_vec(),
                heights,
            }),
        }
    } else {
        let fine = channels.fine(&height, factor, &pool);
        let mut ground = fine.height;
        if args.island_detail > 0.0 {
            let detail_start = Instant::now();
            let amplified = island_amplified(&height, factor, args.island.unwrap_or(7), &pool);
            // How far each sample stands from the water's reach: the samples of the refined
            // cells, the lakes', and those under the shore's band.
            let n = size as usize;
            let side = n - 1;
            let mut near: Vec<bool> = height.data.iter().map(|&h| h < SHORE_SMOOTHING.0).collect();
            for &c in channels.refined() {
                let (i, j) = (c as usize % side, c as usize / side);
                for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    near[(j + dj) * n + i + di] = true;
                }
            }
            for lake in &lakes {
                for j in lake.first[1]..lake.first[1] + lake.size[1] {
                    for i in lake.first[0]..lake.first[0] + lake.size[0] {
                        if lake.covers(i, j) {
                            near[j as usize * n + i as usize] = true;
                        }
                    }
                }
            }
            let distance = forge_procgen::site_distance(size, spacing, |i| near[i], &pool);
            let (fine_size, fine_spacing) = (ground.size as usize, ground.spacing);
            let strength = args.island_detail;
            // What the detail adds where it is whole: its root mean square and its largest.
            let (mut sum, mut count, mut largest) = (0.0f64, 0u64, 0.0f32);
            for j in 0..fine_size {
                for i in 0..fine_size {
                    let d = distance.sample(i as f64 * fine_spacing, j as f64 * fine_spacing);
                    if d >= DETAIL_FADE.1 {
                        let s = j * fine_size + i;
                        let r = amplified.data[s] - ground.data[s];
                        sum += f64::from(r * r);
                        count += 1;
                        largest = largest.max(r.abs());
                    }
                }
            }
            tracing::info!(
                rms_m = %format_args!("{:.3}", (sum / count.max(1) as f64).sqrt()),
                largest_m = %format_args!("{largest:.2}"),
                samples = count,
                "the amplification's detail over the cubic, away from the water"
            );
            pool.par_chunks_mut(&mut ground.data, fine_size, |j, row| {
                let y = j as f64 * fine_spacing;
                for (i, h) in row.iter_mut().enumerate() {
                    let d = distance.sample(i as f64 * fine_spacing, y);
                    let t = ((d - DETAIL_FADE.0) / (DETAIL_FADE.1 - DETAIL_FADE.0)).clamp(0.0, 1.0);
                    let w = strength * t * t * (3.0 - 2.0 * t);
                    *h += w * (amplified.data[j * fine_size + i] - *h);
                }
            });
            detail_ms = detail_start.elapsed().as_millis();
        }
        DrawnGround {
            size: ground.size,
            spacing: ground.spacing,
            heights: Arc::from(ground.data),
            detail: Arc::new(HeightfieldDetail {
                split: fine.split,
                cells: fine.cells,
                heights: fine.heights,
            }),
        }
    };
    tracing::info!(
        drawn_m = drawn.spacing,
        samples = drawn.size,
        detail = args.island_detail,
        refined_cells = drawn.detail.cells.len(),
        fine_vertices = drawn.detail.heights.len(),
        detail_ms,
        ms = start.elapsed().as_millis(),
        "island ground drawn, the river channels carved"
    );
    let drawn = Arc::new(drawn);
    *made = Some((key, drawn.clone()));
    drawn
}

/// One field of a GPU cascade's sample.
type FieldOf = fn(&forge_render::WaterSample) -> f32;

/// The sea's start-up check (issue #105): every cascade the GPU transformed, read back and
/// compared field by field with the CPU's `Ocean::surface` at the same time. Each field's
/// largest difference over its largest magnitude; it passes under 10⁻³, the precision of the
/// half floats the images keep.
fn water_check(
    device: &Arc<forge_gpu::Device>,
    water: &WaterCascades,
    oceans: &[Ocean],
    time: f32,
) -> Result<()> {
    let mut worst = 0.0_f32;
    let mut fields = Vec::new();
    for (c, ocean) in oceans.iter().enumerate() {
        let gpu = water.read_fields(device, c)?;
        let cpu = ocean.surface(f64::from(time));
        let pairs: [(&str, &[f32], FieldOf); 6] = [
            ("height", &cpu.height.data, |s| s.height),
            ("dx", &cpu.dx.data, |s| s.dx),
            ("dy", &cpu.dy.data, |s| s.dy),
            ("slope_x", &cpu.slope_x.data, |s| s.slope_x),
            ("slope_y", &cpu.slope_y.data, |s| s.slope_y),
            ("jacobian", &cpu.jacobian.data, |s| s.jacobian),
        ];
        for (name, reference, get) in pairs {
            let scale = reference.iter().fold(1e-6_f32, |m, v| m.max(v.abs()));
            let error = gpu
                .iter()
                .zip(reference)
                .map(|(g, r)| (get(g) - r).abs())
                .fold(0.0_f32, f32::max);
            worst = worst.max(error / scale);
            fields.push(format!("{c}/{name} {error:.1e} of {scale:.2}"));
        }
    }
    if worst < 1e-3 {
        tracing::info!(time, worst = %format_args!("{worst:.1e}"), fields = %fields.join(", "), "water check passed: the GPU's cascades agree with the CPU's surface");
    } else {
        tracing::warn!(time, worst = %format_args!("{worst:.1e}"), fields = %fields.join(", "), "water check FAILED: a GPU cascade departs from the CPU's surface");
    }
    Ok(())
}

/// Where the camera starts: the island's first view, the gallery's or the city's, or
/// `--view`.
fn start_camera(args: &Args) -> Result<FlyCamera> {
    let mut camera = if args.island.is_some() {
        island_camera(args)
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
    Ok(camera)
}

/// The streamed scene loads the cut from where the camera starts before its first frame (#121):
/// a fixed view then streams nothing, and the capture batch can draw the island's 2 m ground.
fn set_start_view(
    builder: &mut MeshletSceneBuilder,
    ctx: &Context,
    args: &Args,
    origin: forge_render::CellPos,
    camera: &FlyCamera,
) {
    if args.no_lod {
        return; // without the LOD cut nothing is wanted past the roots
    }
    builder.set_start_view(StartView {
        position: origin.offset(camera.position),
        p11: camera.projection(ctx.aspect()).y_axis.y,
        near: camera.near,
        viewport_height: ctx.extent().height,
        lod_threshold_px: args.lod_error,
    });
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
    let camera = FlyCamera {
        position: Vec3::new(0.0, 25.0, (beach + 150.0) as f32),
        pitch: 0.03,
        speed: 60.0,
        ..FlyCamera::default()
    };
    tracing::info!(
        beach_z = %format_args!("{beach:.0}"),
        camera = %format_args!("{:.0},{:.0},{:.0}", camera.position.x, camera.position.y, camera.position.z),
        "island first view (--view takes x,y,z,yaw,pitch in degrees)"
    );
    camera
}

/// The island's props, in the order `build_island` reads them: the island's tiles, the sea around
/// it, the city's boulders (the same cache files as the city's) for the stones in its rivers,
/// then its own stones (#130), granite then limestone; or with `--no-rock-sites`, the city's
/// boulders and rubble for all its rocks.
fn island_props(args: &Args) -> Vec<PropSpec> {
    let mut props = island_tiles(args);
    props.push(sea_prop());
    props.extend(city_props().into_iter().filter(|p| match p.kind {
        PropKind::Boulder { .. } => args.no_rock_sites,
        PropKind::Rubble { .. } => args.no_rock_sites,
        _ => false,
    }));
    if !args.no_rock_sites {
        props.extend(island_stone_props(args.stone_normals));
        props.extend(island_cobble_props(args.stone_normals));
    }
    // The movers' barrel last (#79), after the rocks.
    if args.movers > 0 {
        props.push(barrel_prop());
    }
    props
}

/// The island's stones (#130), granite's then limestone's, eight each (the placement's most
/// per rock): the name, the shape and the half-size in metres. A rock is any of its rock's
/// eight alike, so a shape's count among them is its share: the granite mostly corestones,
/// a slab now and then, a tor in eight; the limestone mostly blocks, and flags.
const ISLAND_STONES: [(&str, StoneShape, [f32; 3]); 16] = [
    (
        "granite-corestone-1",
        StoneShape::Corestone,
        [1.4, 1.0, 1.2],
    ),
    (
        "granite-corestone-2",
        StoneShape::Corestone,
        [1.8, 0.9, 1.1],
    ),
    (
        "granite-corestone-3",
        StoneShape::Corestone,
        [1.0, 0.95, 1.0],
    ),
    (
        "granite-corestone-4",
        StoneShape::Corestone,
        [1.5, 1.2, 1.4],
    ),
    (
        "granite-corestone-5",
        StoneShape::Corestone,
        [1.2, 0.7, 0.9],
    ),
    ("granite-slab-1", StoneShape::Slab, [2.0, 0.45, 1.5]),
    ("granite-slab-2", StoneShape::Slab, [1.5, 0.35, 1.3]),
    ("granite-tor", StoneShape::Tor, [2.0, 1.2, 1.7]),
    ("limestone-block-1", StoneShape::Block, [1.2, 0.8, 1.0]),
    ("limestone-block-2", StoneShape::Block, [1.6, 0.6, 1.1]),
    ("limestone-block-3", StoneShape::Block, [0.9, 0.9, 0.8]),
    ("limestone-block-4", StoneShape::Block, [1.4, 1.1, 1.3]),
    ("limestone-block-5", StoneShape::Block, [1.0, 0.5, 0.7]),
    ("limestone-block-6", StoneShape::Block, [1.7, 1.0, 1.2]),
    ("limestone-flag-1", StoneShape::Block, [1.5, 0.25, 1.2]),
    ("limestone-flag-2", StoneShape::Block, [1.1, 0.2, 0.9]),
];

/// How many of [`ISLAND_STONES`] are granite (the first ones).
const GRANITE_STONES: usize = 8;

/// The stones in the island's rivers (#132, #133), granite's then limestone's: the name, the shape
/// and the half-size, a metre long. Mostly chunks the water broke off, of any proportions, their
/// edges worn round; a round pebble among them.
const ISLAND_COBBLES: [(&str, StoneShape, [f32; 3]); 10] = [
    ("granite-worn-1", StoneShape::Worn, [1.0, 0.7, 0.8]),
    ("granite-worn-2", StoneShape::Worn, [1.0, 0.5, 0.7]),
    ("granite-worn-3", StoneShape::Worn, [1.0, 0.8, 0.9]),
    ("granite-worn-4", StoneShape::Worn, [1.0, 0.45, 0.55]),
    ("granite-worn-5", StoneShape::Worn, [1.0, 0.65, 0.6]),
    ("granite-pebble", StoneShape::Cobble, [1.0, 0.75, 0.85]),
    ("limestone-worn-1", StoneShape::Worn, [1.0, 0.55, 0.8]),
    ("limestone-worn-2", StoneShape::Worn, [1.0, 0.7, 0.65]),
    ("limestone-worn-3", StoneShape::Worn, [1.0, 0.4, 0.75]),
    ("limestone-pebble", StoneShape::Cobble, [1.0, 0.75, 0.8]),
];

/// How many of [`ISLAND_COBBLES`] are granite (the first ones).
const GRANITE_COBBLES: usize = 6;

/// The props of [`ISLAND_COBBLES`], a metre long.
fn island_cobble_props(normals: f32) -> Vec<PropSpec> {
    ISLAND_COBBLES
        .iter()
        .enumerate()
        .map(|(i, &(name, shape, size))| PropSpec {
            name: name.to_owned(),
            kind: PropKind::Stone(Stone {
                seed: 150 + i as u64,
                shape,
                size,
                segments: 64,
                normal_weight: normals,
            }),
        })
        .collect()
}

/// The island's rocks from its rock sites (#130), unless `--instances` says otherwise.
const ISLAND_ROCKS: u32 = 60_000;

/// The props of [`ISLAND_STONES`].
fn island_stone_props(normals: f32) -> Vec<PropSpec> {
    ISLAND_STONES
        .iter()
        .enumerate()
        .map(|(i, &(name, shape, size))| PropSpec {
            name: name.to_owned(),
            kind: PropKind::Stone(Stone {
                seed: 130 + i as u64,
                shape,
                size,
                segments: if shape == StoneShape::Block { 96 } else { 80 },
                normal_weight: normals * size.iter().copied().fold(0.0, f32::max),
            }),
        })
        .collect()
}

/// A metal drum, 0.6 m across and 0.88 m long with two rolling hoops: the movers of `--movers`
/// (#79). Its axis is the lathe's, +y from its bottom's centre.
fn barrel_prop() -> PropSpec {
    let r = BARREL_RADIUS;
    PropSpec {
        name: "barrel".to_owned(),
        kind: PropKind::Lathe(Lathe {
            profile: vec![
                (0.0, 0.0),
                (r - 0.02, 0.0),
                (r, 0.02),
                (r, 0.28),
                (r + 0.012, 0.3),
                (r, 0.32),
                (r, 0.56),
                (r + 0.012, 0.58),
                (r, 0.6),
                (r, BARREL_LENGTH - 0.02),
                (r - 0.02, BARREL_LENGTH),
                (0.0, BARREL_LENGTH),
            ],
            around: 96,
            along: 64,
            flutes: 0,
            flute_depth: 0.0,
            flute_span: (0.0, 0.0),
        }),
    }
}

/// The barrel's radius and length, metres.
const BARREL_RADIUS: f32 = 0.3;
const BARREL_LENGTH: f32 = 0.88;
/// The slowest the water carries a barrel, metres a second (where the river slows into a lake).
const BARREL_SLOWEST: f32 = 0.2;
/// One barrel in this many on a river is moored where it is, the stream running past it (#107).
const BARREL_MOORED: u32 = 10;
/// The rivers that carry barrels: the largest.
const BARREL_RIVERS: usize = 4;
/// The towed barrel (#107): metres a second round its circle on the largest lake, and the
/// circle's radius at most, metres.
const TOW_SPEED: f32 = 2.5;
const TOW_RADIUS: f32 = 20.0;
/// The dropped barrel (#107's splashes), one more than `--movers` when it has two or more: over
/// the middle of the towed barrel's circle, every period it hangs a while this far over its
/// floating level, falls, bobs, and is lifted out again from the given second.
const DROP_PERIOD: f64 = 10.0;
const DROP_HEIGHT: f32 = 3.0;
const DROP_HANG: f64 = 2.0;
const DROP_LIFT: f64 = 6.5;
/// The air near the water, a share of the sea's wind at 10 m: the spray drifts in it.
const SPRAY_WIND: f32 = 0.15;

/// The movers of `--movers` (#79): barrels carried down the island's largest rivers at the
/// water's speed, spread along each river's course and starting over at its head once past its
/// mouth; one in ten moored where it is, bobbing as the stream runs past (#107). The last one is
/// towed round a circle on the largest lake, faster than its waves (#107's wakes). They float
/// with their axis across their way, half under the water's level, rolling as they go. With two
/// or more, one more is dropped into the middle of the towed one's circle again and again
/// (#107's splashes).
struct Barrels {
    rivers: Vec<BarrelCourse>,
    count: u32,
    /// The towed barrel's circle: its centre (world x, z), radius and the lake's level.
    towed: Option<(Vec2, f32, f32)>,
    /// Where the dropped barrel falls (world x, z) and the level of the water it falls into.
    dropped: Option<(Vec2, f32)>,
}

/// A river's course as the barrels follow it.
struct BarrelCourse {
    /// Its points: world x and z in the sea's frame, and the water's level.
    points: Vec<(Vec2, f32)>,
    /// Metres along it to each point.
    along: Vec<f32>,
    /// Seconds a barrel carried at the water's speed takes to reach each point.
    times: Vec<f32>,
    /// How much of the river is drawn at each point: under a half, a lake's still water.
    fades: Vec<f32>,
}

impl Barrels {
    fn new(rivers: &[Vec<WaterRiverPoint>], lakes: &[WaterLake], spacing: f32, count: u32) -> Self {
        // The largest rivers are uploaded last.
        let rivers = rivers
            .iter()
            .rev()
            .take(BARREL_RIVERS)
            .map(|r| {
                let points: Vec<(Vec2, f32)> = r
                    .iter()
                    .map(|p| (Vec2::from(p.position), p.level))
                    .collect();
                let (mut along, mut times) = (vec![0.0_f32], vec![0.0_f32]);
                for (pair, p) in points.windows(2).zip(r.windows(2)) {
                    let metres = pair[0].0.distance(pair[1].0);
                    let speed = (0.5 * (p[0].speed + p[1].speed)).max(BARREL_SLOWEST);
                    along.push(along.last().unwrap() + metres);
                    times.push(times.last().unwrap() + metres / speed);
                }
                BarrelCourse {
                    points,
                    along,
                    times,
                    fades: r.iter().map(|p| p.fade).collect(),
                }
            })
            .collect();
        let towed = (count > 1)
            .then(|| {
                lakes
                    .iter()
                    .max_by_key(|l| l.mask.iter().filter(|&&m| m).count())
            })
            .flatten()
            .map(|lake| {
                let (centre, room) = lake_middle(lake, spacing);
                (centre, (0.6 * room).min(TOW_RADIUS), lake.level)
            });
        // Without a lake it waits far under the ground: the movers' table counts it all the same.
        let dropped = (count > 1)
            .then(|| towed.map_or((Vec2::ZERO, -1000.0), |(centre, _, level)| (centre, level)));
        Self {
            rivers,
            count,
            towed,
            dropped,
        }
    }

    /// The movers the barrels take in the table: `--movers`, and the dropped barrel.
    fn movers(count: u32) -> u32 {
        count + u32::from(count > 1)
    }

    /// The dropped barrel `time` seconds in: its transform, its centre, its speed upwards, and
    /// where its drop cycle stands (the cycle's number and the seconds into it).
    fn dropped_pose(&self, time: f64) -> Option<(MoverTransform, Vec3, f32, (u64, f64))> {
        let (centre, level) = self.dropped?;
        let cycle = (time / DROP_PERIOD).floor().max(0.0);
        let into = time - cycle * DROP_PERIOD;
        let rest = level - 0.05;
        let (height, falls_at) = (
            f64::from(DROP_HEIGHT),
            (2.0 * f64::from(DROP_HEIGHT) / 9.81).sqrt(),
        );
        let (y, rise) = if into < DROP_HANG {
            (f64::from(rest) + height, 0.0)
        } else if into < DROP_HANG + falls_at {
            // Falling.
            let s = into - DROP_HANG;
            (f64::from(rest) + height - 0.5 * 9.81 * s * s, -9.81 * s)
        } else if into < DROP_LIFT {
            // Plunging and bobbing back up, damped.
            let (u, v) = (into - DROP_HANG - falls_at, 9.81 * falls_at);
            let (omega, damping) = (std::f64::consts::TAU * 0.8, 3.0);
            let decay = (-damping * u).exp();
            (
                f64::from(rest) - v / omega * decay * (omega * u).sin(),
                -v * decay * ((omega * u).cos() - damping / omega * (omega * u).sin()),
            )
        } else {
            // Lifted out again, smoothly.
            let span = DROP_PERIOD - DROP_LIFT;
            let w = (into - DROP_LIFT) / span;
            (
                f64::from(rest) + height * w * w * (3.0 - 2.0 * w),
                height * 6.0 * w * (1.0 - w) / span,
            )
        };
        let rotation = Quat::from_rotation_arc(Vec3::Y, Vec3::X);
        let middle = Vec3::new(centre.x, y as f32, centre.y);
        let transform = MoverTransform {
            position: middle - rotation * Vec3::new(0.0, 0.5 * BARREL_LENGTH, 0.0),
            rotation,
            scale: 1.0,
        };
        Some((transform, middle, rise as f32, (cycle as u64, into)))
    }

    /// Where the barrels make the water splash `time` seconds in (#107): the dropped barrel
    /// meeting the water, and the drops running off it as it is lifted out; the towed barrel's
    /// bow.
    fn splashes(&self, time: f64, out: &mut Vec<SplashSource>) {
        if let (Some((_, middle, rise, (cycle, _))), Some((_, level))) =
            (self.dropped_pose(time), self.dropped)
        {
            let seed = (cycle as u32).wrapping_mul(0x9e37_79b9) ^ 0xd209;
            // Its bottom meets the water a little before its centre reaches its rest.
            let fall = 2.0 * f64::from(DROP_HEIGHT - 0.05 - BARREL_RADIUS) / 9.81;
            let meets = cycle as f64 * DROP_PERIOD + DROP_HANG + fall.sqrt();
            if (0.0..=1.0).contains(&(time - meets)) {
                // Lying across its fall: the circle of its outline's area.
                let radius = (2.0 * BARREL_RADIUS * BARREL_LENGTH / std::f32::consts::PI).sqrt();
                out.push(SplashSource::Impact {
                    position: Vec3::new(middle.x, level, middle.z),
                    velocity: Vec3::new(0.0, -9.81 * fall.sqrt() as f32, 0.0),
                    radius,
                    density: 0.5,
                    time: meets as f32,
                    seed,
                });
            }
            // Out of the water and rising: drops run off its underside, fewer as it climbs.
            let above = middle.y - BARREL_RADIUS - level;
            if above > 0.0 && rise > 0.0 {
                out.push(SplashSource::Drip {
                    position: middle - Vec3::new(0.0, BARREL_RADIUS, 0.0),
                    spread: Vec3::new(0.5 * BARREL_LENGTH, 0.0, 0.0),
                    velocity: Vec3::new(0.0, rise, 0.0),
                    level,
                    rate: 60.0 * (-above / 0.5).exp(),
                    seed: seed ^ 0xd419,
                });
            }
        }
        if self.towed.is_some() {
            let (_, centre, velocity, _) = self.pose(self.count - 1, time);
            let ahead = velocity.normalize_or_zero();
            out.push(SplashSource::Bow {
                bow: Vec3::new(centre.x, centre.y + 0.05, centre.z)
                    + BARREL_RADIUS * Vec3::new(ahead.x, 0.0, ahead.y),
                velocity,
                beam: BARREL_LENGTH,
                length: 2.0 * BARREL_RADIUS,
                seed: 0x70ed,
            });
        }
    }

    /// The barrels on the rivers: all but the towed one.
    fn on_rivers(&self) -> u32 {
        self.count - u32::from(self.towed.is_some())
    }

    /// Whether barrel `k` is the towed one.
    fn is_towed(&self, k: u32) -> bool {
        self.towed.is_some() && k == self.count - 1
    }

    /// Whether barrel `k` is moored.
    fn moored(&self, k: u32) -> bool {
        !self.is_towed(k) && (k / self.rivers.len() as u32) % BARREL_MOORED == BARREL_MOORED / 2
    }

    /// Where river barrel `k` is `time` seconds in: its course's segment, how far along it
    /// (0..1) and the water's speed there. A moored barrel stays where its share of the course's
    /// length puts it, a carried one goes where its share of the course's time does.
    fn place(&self, k: u32, time: f64) -> (usize, f32, f32) {
        let per_river = self.on_rivers().div_ceil(self.rivers.len() as u32).max(1);
        let course = &self.rivers[k as usize % self.rivers.len()];
        let start = (k / self.rivers.len() as u32) as f32 / per_river as f32;
        let last = course.points.len() - 1;
        // Its point along the course, in metres or in seconds, and the table that measures it.
        let (at, table) = if self.moored(k) {
            (start * course.along[last], &course.along)
        } else {
            let total = f64::from(course.times[last]);
            let at = (f64::from(start) * total + time).rem_euclid(total);
            (at as f32, &course.times)
        };
        let i = table.partition_point(|&a| a <= at).clamp(1, last) - 1;
        let t = (at - table[i]) / (table[i + 1] - table[i]).max(1e-3);
        let metres = course.along[i + 1] - course.along[i];
        let seconds = (course.times[i + 1] - course.times[i]).max(1e-3);
        (i, t.clamp(0.0, 1.0), metres / seconds)
    }

    /// Whether barrel `k` floats in still water `time` seconds in: towed on its lake, or where
    /// its river has faded into one.
    fn still(&self, k: u32, time: f64) -> bool {
        if self.is_towed(k) {
            return true;
        }
        let course = &self.rivers[k as usize % self.rivers.len()];
        let (i, t, _) = self.place(k, time);
        course.fades[i] + t * (course.fades[i + 1] - course.fades[i]) < 0.5
    }

    /// Barrel `k` `time` seconds in: its transform (relative to the scene's origin, the sea's
    /// frame), its centre, its velocity (world x and z) and its speed upwards.
    fn pose(&self, k: u32, time: f64) -> (MoverTransform, Vec3, Vec2, f32) {
        // Its place on its way, its way's level and heading, its speed and how far it has rolled.
        let (flat, level, down, speed, rolled) = match self.towed {
            Some((centre, radius, level)) if self.is_towed(k) => {
                let angle =
                    (f64::from(TOW_SPEED / radius) * time).rem_euclid(std::f64::consts::TAU) as f32;
                let out = Vec2::new(angle.cos(), angle.sin());
                let down = Vec2::new(-out.y, out.x);
                let rolled = (f64::from(TOW_SPEED) * time).rem_euclid(1e4) as f32;
                (centre + radius * out, level, down, TOW_SPEED, rolled)
            }
            _ => {
                let course = &self.rivers[k as usize % self.rivers.len()];
                let (i, t, stream) = self.place(k, time);
                let speed = if self.moored(k) { 0.0 } else { stream };
                let s = course.along[i] + t * (course.along[i + 1] - course.along[i]);
                let (a, b) = (course.points[i], course.points[i + 1]);
                let down = (b.0 - a.0).normalize_or(Vec2::X);
                (a.0.lerp(b.0, t), a.1 + (b.1 - a.1) * t, down, speed, s)
            }
        };
        let across = Vec3::new(-down.y, 0.0, down.x);
        // Bobbing a little, out of step with one another, and rolling as they drift.
        let phase = time as f32 * 1.3 + k as f32 * 2.1;
        let rotation = Quat::from_rotation_arc(Vec3::Y, across)
            * Quat::from_rotation_y(rolled / (2.0 * BARREL_RADIUS))
            * Quat::from_rotation_x(0.05 * phase.sin());
        let centre = Vec3::new(flat.x, level - 0.05 + 0.03 * (1.7 * phase).sin(), flat.y);
        let rise = 0.03 * 1.7 * 1.3 * (1.7 * phase).cos();
        let transform = MoverTransform {
            position: centre - rotation * Vec3::new(0.0, 0.5 * BARREL_LENGTH, 0.0),
            rotation,
            scale: 1.0,
        };
        (transform, centre, down * speed, rise)
    }

    /// Their transforms `time` seconds in, relative to the scene's origin (the sea's frame).
    fn transforms(&self, time: f64) -> Vec<MoverTransform> {
        (0..self.count)
            .map(|k| self.pose(k, time).0)
            .chain(self.dropped_pose(time).map(|d| d.0))
            .collect()
    }

    /// The barrels nearest `camera` (world x and z) as the rivers' water sees them (#107):
    /// their outline at the water's level, about a circle half their length across, and their
    /// velocity.
    fn floaters(&self, time: f64, camera: Vec2) -> Vec<WaterFloater> {
        let mut near: Vec<(f32, WaterFloater)> = (0..self.count)
            .map(|k| {
                let (_, centre, velocity, _) = self.pose(k, time);
                let flat = Vec2::new(centre.x, centre.z);
                let floater = WaterFloater {
                    position: flat.to_array(),
                    waterline: 0.5 * BARREL_LENGTH,
                    velocity: velocity.to_array(),
                };
                (flat.distance_squared(camera), floater)
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.into_iter()
            .take(MAX_FLOATERS)
            .map(|(_, f)| f)
            .collect()
    }

    /// The barrels in still water nearest `camera` (world x and z), making waves (#107).
    fn wakes(&self, time: f64, camera: Vec2) -> Vec<WaterWake> {
        let mut near: Vec<(f32, WaterWake)> = (0..self.count)
            .filter(|&k| self.still(k, time))
            .map(|k| {
                let (_, centre, velocity, rise) = self.pose(k, time);
                let flat = Vec2::new(centre.x, centre.z);
                let wake = WaterWake {
                    position: flat.to_array(),
                    waterline: 0.5 * BARREL_LENGTH,
                    velocity: velocity.to_array(),
                    rise,
                };
                (flat.distance_squared(camera), wake)
            })
            .collect();
        // The dropped barrel while it is in the water: going in, it pushes a ring out.
        if let (Some((_, middle, rise, _)), Some((_, level))) =
            (self.dropped_pose(time), self.dropped)
            && (-1.0..=BARREL_RADIUS + 0.05).contains(&(middle.y - level))
        {
            let flat = Vec2::new(middle.x, middle.z);
            let wake = WaterWake {
                position: flat.to_array(),
                waterline: 0.5 * BARREL_LENGTH,
                velocity: [0.0; 2],
                rise,
            };
            near.push((flat.distance_squared(camera), wake));
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.into_iter().take(MAX_WAKES).map(|(_, w)| w).collect()
    }
}

/// The point of `lake` farthest from its shore (world x and z) and how far that is, metres: the
/// mask's samples (`spacing` apart) by their distance in samples to the nearest dry one, eight
/// neighbours a step.
fn lake_middle(lake: &WaterLake, spacing: f32) -> (Vec2, f32) {
    let [w, h] = lake.size.map(|s| s as usize);
    let mut steps = vec![u32::MAX; w * h];
    let mut queue = std::collections::VecDeque::new();
    for (i, &wet) in lake.mask.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        if !wet || x == 0 || y == 0 || x == w - 1 || y == h - 1 {
            steps[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        for (dx, dy) in [
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                continue;
            }
            let n = ny as usize * w + nx as usize;
            if steps[n] == u32::MAX {
                steps[n] = steps[i] + 1;
                queue.push_back(n);
            }
        }
    }
    let (best, &most) = steps.iter().enumerate().max_by_key(|&(_, s)| *s).unwrap();
    let at = Vec2::new((best % w) as f32, (best / w) as f32) * spacing + Vec2::from(lake.origin);
    (at, most as f32 * spacing)
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
            source: Arc::new(|| Arc::from(vec![0.0; (SAMPLES * SAMPLES) as usize])),
            detail: None,
            window: None,
        }),
    }
}

/// The island (`docs/demos/island.md`): its heightfield cooked (or loaded) as the one
/// instance of the scene, on the ground's layered material with rock where the ground is
/// steep or high and grass elsewhere.
fn build_island(
    ctx: &Context,
    args: &Args,
    cooked: Cooked,
    camera: &FlyCamera,
) -> Result<MeshletScene> {
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
                sand_below: SAND_BELOW,
            }),
        },
        texels,
    );
    tracing::info!(
        texels,
        ms = layers_start.elapsed().as_millis(),
        "island layer map"
    );
    // The moisture, the rivers and the lakes from the drawn field's drainage.
    let rivers_start = Instant::now();
    let flow = forge_procgen::drain(&height, 0.0, &TaskPool::client());
    // First the grass by its moisture (the topographic wetness index, blurred over 32 m): the
    // driest quarter dry grass on the ridges, the wettest quarter lush in the valley bottoms.
    let wetness = forge_procgen::wetness(&height, &flow, 4);
    let (dried, greened) = forge_procgen::paint_moisture(
        &mut layers,
        &wetness,
        island_layer::GRASS,
        (island_layer::DRY_GRASS, 0.25),
        (island_layer::LUSH_GRASS, 0.25),
    );
    // The rivers run in the channels carved for them, which the island's mesh draws (#105).
    // With the water the map paints no bed: a pixel blends the four texels around it and the
    // lookup wanders by one, so a bed's 4 m texels showed up to 8 m onto the banks, wider than
    // most of the rivers (#114); the water turns the ground it covers into its bed itself, per
    // pixel (`fresh_water` in `water.slang`). Without it, their stand-in painted at the
    // water's width.
    let IslandWater {
        ribbons,
        channels,
        lakes: lake_waters,
    } = island_water(&height);
    // The beaches by the coast (#128): shingle on the headlands and under steep land, pale
    // sand in the bays and by the rivers' mouths.
    if !args.no_beach_types {
        let beaches_start = Instant::now();
        let settings = island_settings(args).0;
        let mouths: Vec<[f64; 2]> = ribbons
            .iter()
            .filter_map(|r| {
                let p = r.points[forge_procgen::sea_mouth(&r.points)?].position;
                Some([f64::from(p[0]), f64::from(p[1])])
            })
            .collect();
        let beaches = forge_procgen::paint_beaches(
            &mut layers,
            &height,
            &|x, y| forge_procgen::island::island_hardness(&settings, x, y),
            &mouths,
            forge_procgen::BeachLayers {
                sand: island_layer::SAND,
                sea: island_layer::SEABED,
                shingle: island_layer::SHINGLE,
                // No black sand: the island's hard rock is no basalt (the owner, 2026-10-02).
                black: None,
            },
            &forge_procgen::BeachRule::default(),
        );
        let km = |k: usize| format!("{:.1}", beaches.coast_m[k] / 1000.0);
        // A view of each: the block of 256 m with the most of it, from 70 m out at sea and 18 m
        // up, looking back at its nearest texel to the block's middle.
        let size = layers.size;
        let cell = layers.spacing;
        let half_m = height.extent() * 0.5;
        let views: Vec<String> = [island_layer::SAND, island_layer::SHINGLE]
            .iter()
            .filter_map(|&layer| {
                let block = 64;
                let blocks = size / block;
                let (bx, by) = (0..blocks * blocks)
                    .map(|b| (b % blocks, b / blocks))
                    .max_by_key(|&(bx, by)| {
                        (0..block * block)
                            .filter(|t| {
                                layers.get(bx * block + t % block, by * block + t / block) == layer
                            })
                            .count()
                    })?;
                let middle = [
                    (bx * block + block / 2) as f64,
                    (by * block + block / 2) as f64,
                ];
                let (tx, ty) = (0..block * block)
                    .map(|t| (bx * block + t % block, by * block + t / block))
                    .filter(|&(x, y)| layers.get(x, y) == layer)
                    .min_by(|a, b| {
                        let d = |p: (u32, u32)| {
                            (f64::from(p.0) - middle[0]).hypot(f64::from(p.1) - middle[1])
                        };
                        d(*a).total_cmp(&d(*b))
                    })?;
                let at = [(f64::from(tx) + 0.5) * cell, (f64::from(ty) + 0.5) * cell];
                // Out to sea: down the ground's slope there.
                let step = 8.0;
                let down = [
                    height.sample(at[0] - step, at[1]) - height.sample(at[0] + step, at[1]),
                    height.sample(at[0], at[1] - step) - height.sample(at[0], at[1] + step),
                ];
                let len = f64::from(down[0].hypot(down[1])).max(1e-6);
                let out = [f64::from(down[0]) / len, f64::from(down[1]) / len];
                let eye = [at[0] + 70.0 * out[0], at[1] + 70.0 * out[1]];
                let yaw = out[0].atan2(out[1]).to_degrees();
                let pitch = (-(18.0_f64).atan2(70.0)).to_degrees();
                Some(format!(
                    "{:.0},18,{:.0},{yaw:.1},{pitch:.1}",
                    eye[0] - half_m,
                    eye[1] - half_m
                ))
            })
            .collect();
        tracing::info!(
            sand_km = %km(0),
            shingle_km = %km(1),
            texels = ?beaches.texels,
            under_sea = beaches.under_sea,
            ms = beaches_start.elapsed().as_millis(),
            views = %views.join("  "),
            "the island's beaches: sand, shingle (#128, --view)"
        );
    }
    let painted = if args.water() {
        0
    } else {
        forge_procgen::paint_beds(&mut layers, &ribbons, island_layer::STREAM, 0.5)
    };
    // Along them the banks' reeds and shrubs (D-041's riparian strip): the grasses within 6 m
    // plus two of the river's widths of its water (and under 2.5 m the sand's contour still
    // takes them, per pixel).
    let banks = forge_procgen::paint_banks(
        &mut layers,
        &ribbons,
        &[
            island_layer::GRASS,
            island_layer::DRY_GRASS,
            island_layer::LUSH_GRASS,
        ],
        island_layer::RIVERBANK,
        RIPARIAN_STRIP,
    );
    // And its lakes of a hectare or more: with the water, their beds of silt wherever the
    // lakes' planes stand a metre or more over the ground (their shallows, like the rivers,
    // the ground under the water's own bed); without it, on the stream's layer.
    let lakes = island_lakes(&height, &flow);
    let lake_texels = if args.water() {
        forge_procgen::paint_lake_beds(
            &mut layers,
            &height,
            &|x, y| channels.height_at(&height, x, y) + LAKEBED_UNDER,
            &lake_waters,
            island_layer::LAKEBED,
        )
    } else {
        forge_procgen::paint_lakes(
            &mut layers,
            &lakes,
            (height.size, height.spacing),
            island_layer::STREAM,
            10_000.0,
            0.5,
        )
    };
    // The rivers' deltas (#120): the pale sand they lay on the lakes' floors in front of their
    // mouths, over the mud, under the water.
    let fan_texels = if args.water() {
        forge_procgen::paint_fans(
            &mut layers,
            &ribbons,
            &|x, y| channels.height_at(&height, x, y),
            island_layer::LAKE_SAND,
        )
    } else {
        0
    };
    // The bars in the large mouths at the sea (#127): the beach's sand.
    let bar_texels = if args.water() {
        forge_procgen::paint_bars(&mut layers, &ribbons, island_layer::SAND)
    } else {
        0
    };
    // The steep ground's scrub (#118): plants on the wetter rock, the hollows and the valleys'
    // sides, in patches; the dry spurs and the cliffs stay bare.
    let scrubbed = forge_procgen::paint_scrub(
        &mut layers,
        &height,
        &wetness,
        (island_layer::ROCK, island_layer::SCRUB),
        &forge_procgen::ScrubRule::default(),
    );
    // The steeper rivers' valleys (#118): gravel in their beds and beside their water, scree at
    // the foot of their walls, scrub on the rock of the walls above.
    let valleys = forge_procgen::paint_valley_ground(
        &mut layers,
        &height,
        &ribbons,
        &[
            island_layer::GRASS,
            island_layer::DRY_GRASS,
            island_layer::LUSH_GRASS,
            island_layer::RIVERBANK,
            island_layer::ROCK,
            island_layer::SCRUB,
        ],
        &forge_procgen::ValleyGround {
            gravel: island_layer::GRAVEL,
            scree: island_layer::SCREE,
            scrub: island_layer::SCRUB,
            rock: island_layer::ROCK,
            ..forge_procgen::ValleyGround::default()
        },
    );
    tracing::info!(
        rivers = ribbons.len(),
        texels = painted,
        bank_texels = banks,
        scrub_texels = %format_args!("{scrubbed} in hollows, {} on valley walls", valleys.scrub),
        gravel_texels = valleys.gravel,
        scree_texels = valleys.scree,
        lakes = lakes
            .lakes
            .iter()
            .filter(|l| l.area(height.spacing) >= 10_000.0 && l.level > 0.5)
            .count(),
        lake_texels,
        fan_texels,
        bar_texels,
        dry_texels = dried,
        lush_texels = greened,
        ms = rivers_start.elapsed().as_millis(),
        "island moisture, rivers and lakes"
    );
    // The rock by the island's geology (D-042, #129), after the rules that read the rock: the
    // hills' granite, the low ground's limestone, and karst on the limestone's dry ground.
    if !args.no_rock_types {
        let geology_start = Instant::now();
        let rocks = forge_procgen::paint_geology(
            &mut layers,
            &height,
            forge_procgen::GeologyLayers {
                rock: island_layer::ROCK,
                limestone: island_layer::LIMESTONE,
                dry_grass: island_layer::DRY_GRASS,
                karst: island_layer::KARST,
            },
            &forge_procgen::GeologyRule::default(),
        );
        // A view of each: the block of 256 m with the most of it, from 150 m down the ground's
        // slope from its nearest texel to the block's middle and 50 m over it, looking back.
        let half_m = height.extent() * 0.5;
        let view = |layer: u8| -> Option<String> {
            let (block, cell) = (64, layers.spacing);
            let blocks = layers.size / block;
            let mut count = vec![0_u32; (blocks * blocks) as usize];
            for (t, &l) in layers.data.iter().enumerate() {
                if l == layer {
                    let (x, y) = (t as u32 % layers.size, t as u32 / layers.size);
                    count[((y / block) * blocks + x / block) as usize] += 1;
                }
            }
            let b = (0..count.len()).max_by_key(|&b| count[b])?;
            let (bx, by) = (b as u32 % blocks, b as u32 / blocks);
            let middle = (
                (bx * block + block / 2) as f64,
                (by * block + block / 2) as f64,
            );
            let (tx, ty) = (0..block * block)
                .map(|t| (bx * block + t % block, by * block + t / block))
                .filter(|&(x, y)| layers.get(x, y) == layer)
                .min_by(|a, b| {
                    let d = |p: (u32, u32)| {
                        (f64::from(p.0) - middle.0).hypot(f64::from(p.1) - middle.1)
                    };
                    d(*a).total_cmp(&d(*b))
                })?;
            let at = ((f64::from(tx) + 0.5) * cell, (f64::from(ty) + 0.5) * cell);
            let ground = f64::from(height.sample(at.0, at.1));
            let step = 8.0;
            let down = (
                f64::from(height.sample(at.0 - step, at.1) - height.sample(at.0 + step, at.1)),
                f64::from(height.sample(at.0, at.1 - step) - height.sample(at.0, at.1 + step)),
            );
            let len = down.0.hypot(down.1).max(1e-6);
            let out = (down.0 / len, down.1 / len);
            let yaw = out.0.atan2(out.1).to_degrees();
            let pitch = (-(50.0_f64).atan2(150.0)).to_degrees();
            Some(format!(
                "{:.0},{:.0},{:.0},{yaw:.1},{pitch:.1}",
                at.0 + 150.0 * out.0 - half_m,
                ground + 50.0,
                at.1 + 150.0 * out.1 - half_m
            ))
        };
        let views: Vec<String> = [
            island_layer::ROCK,
            island_layer::LIMESTONE,
            island_layer::KARST,
        ]
        .iter()
        .filter_map(|&l| view(l))
        .collect();
        tracing::info!(
            granite_texels = rocks.granite,
            limestone_texels = rocks.limestone,
            karst_texels = rocks.karst,
            ms = geology_start.elapsed().as_millis(),
            views = %views.join("  "),
            "the island's rocks: granite, limestone, karst (D-042, #129, --view)"
        );
    }
    // Where the loose rocks lie, and which rock they are (#130): the map the placement draws
    // them from.
    let sites = (!args.no_rock_sites).then(|| {
        let sites_start = Instant::now();
        let (map, stats) = forge_procgen::rock_sites(
            &height,
            &layers,
            &forge_procgen::SiteLayers {
                scree: island_layer::SCREE,
                karst: island_layer::KARST,
                none: vec![
                    island_layer::SAND,
                    island_layer::SEABED,
                    island_layer::STREAM,
                    island_layer::LAKEBED,
                    island_layer::GRAVEL,
                    island_layer::LAKE_SAND,
                    island_layer::SHINGLE,
                ],
            },
            &forge_procgen::GeologyRule::default(),
            &forge_procgen::RockSiteRule::default(),
        );
        (map, stats, sites_start.elapsed().as_millis())
    });
    builder.set_ray_traced(!args.no_shadows);
    // The ground's tiles, then the sea, then the rocks (`island_props`).
    let tiles = (ISLAND_TILES * ISLAND_TILES) as usize;
    let (tile_ids, sea) = (&ids[..tiles], ids[tiles]);
    // The tiles cut for the rays as the one mesh they were (#106).
    builder.set_ray_group(tile_ids, forge_render::raytrace::TERRAIN_BUDGET);
    let mut materials = CityMaterials::new(&ctx.device)?;
    materials.island_ground(
        &layers.data,
        texels,
        extent,
        !args.no_rock_types,
        !args.no_rock_sites,
    )?;
    materials.apply(&mut builder, &props, &ids);
    let mut layout = CityLayout::island(args.instances.unwrap_or(if sites.is_some() {
        ISLAND_ROCKS
    } else {
        300_000
    }));
    if let Some((_, stats, _)) = &sites {
        layout.rocks = placement::RockRule::Sites {
            second: stats.limestone_share() as f32,
        };
    }
    layout.origin = scene_origin(args);
    builder.set_origin(layout.origin);
    if streamed {
        set_start_view(&mut builder, ctx, args, layout.origin, camera);
    }
    // The ground first: should the cull's work list overflow (`--no-lod`), the instances last in
    // the table are the ones dropped.
    for &tile in tile_ids {
        builder.add_instance(tile, Mat4::IDENTITY);
    }
    // The stand-in sea, unless the water surface draws the sea (issue #105).
    if !args.water() {
        builder.add_instance(sea, Mat4::IDENTITY);
    }
    // The stones in the rivers (#105), scaled to each stone, standing on the bed: the island's
    // cobbles (#132), or with `--no-rock-sites` the city's boulders.
    let boulders: Vec<(MeshId, f32)> = props
        .iter()
        .zip(&ids)
        .filter_map(|(p, &id)| match p.kind {
            PropKind::Boulder { radius, .. } => Some((id, radius)),
            _ => None,
        })
        .collect();
    let cobble = |names: &[(&str, StoneShape, [f32; 3])]| -> Vec<MeshId> {
        names
            .iter()
            .map(|(name, _, _)| ids[props.iter().position(|p| p.name == *name).expect("cobble")])
            .collect()
    };
    let cobbles = (!args.no_rock_sites).then(|| {
        (
            cobble(&ISLAND_COBBLES[..GRANITE_COBBLES]),
            cobble(&ISLAND_COBBLES[GRANITE_COBBLES..]),
        )
    });
    let geology = forge_procgen::GeologyRule::default();
    let mut stones = island_stones(&height, &ribbons, &channels);
    // And beside the steeper rivers' water, on their gravel (#118), clear of the water.
    let banked = forge_procgen::bank_stones(&ribbons, &channels, &height, RIVER_STONES ^ 0xba);
    stones.extend_from_slice(&banked);
    // On the scree at the foot of their walls, the rubble piles scaled down to broken rock: a
    // pile on a third of its texels, 0.2 to 0.4 of its size, sunk a little (with `--no-rock-sites`:
    // the rock sites put the island's own stones there, #130).
    // The ground as the tiles draw it (#106): finer than the field, its cubic.
    let drawn = island_drawn(args);
    let factor = island_factor(args);
    let drawn_at = |x: f64, y: f64| {
        if factor > 1 {
            drawn.height_at(x, y)
        } else {
            channels.height_at(&height, x, y)
        }
    };
    let rubble: Vec<(MeshId, Mat4)> = {
        let piles: Vec<MeshId> = props
            .iter()
            .zip(&ids)
            .filter_map(|(p, &id)| matches!(p.kind, PropKind::Rubble { .. }).then_some(id))
            .collect();
        let cell = f64::from(extent) / f64::from(texels);
        let half = f64::from(extent) * 0.5;
        layers
            .data
            .iter()
            .enumerate()
            .filter(|&(_, &layer)| layer == island_layer::SCREE && !piles.is_empty())
            .filter_map(|(i, _)| {
                let (x, y) = ((i as u32 % texels) as i32, (i as u32 / texels) as i32);
                let draw = |salt: u64| {
                    forge_core::hash::unit_f32(forge_core::hash::hash_cell2(
                        SCREE_RUBBLE ^ salt,
                        x,
                        y,
                    ))
                };
                (draw(0) < 1.0 / 3.0).then(|| {
                    let at = [
                        (f64::from(x) + draw(1) as f64) * cell,
                        (f64::from(y) + draw(2) as f64) * cell,
                    ];
                    let ground = drawn_at(at[0], at[1]) as f32;
                    let mesh = piles[(draw(3) * piles.len() as f32) as usize % piles.len()];
                    (
                        mesh,
                        Mat4::from_scale_rotation_translation(
                            Vec3::splat(0.2 + 0.2 * draw(4)),
                            Quat::from_rotation_y(draw(5) * std::f32::consts::TAU),
                            Vec3::new((at[0] - half) as f32, ground - 0.1, (at[1] - half) as f32),
                        ),
                    )
                })
            })
            .collect()
    };
    for (mesh, transform) in &rubble {
        builder.add_instance(*mesh, *transform);
    }
    tracing::info!(
        bank_stones = banked.len(),
        rubble_on_scree = rubble.len(),
        "the steep valleys' stones and rubble"
    );
    let half = 0.5 * extent;
    for stone in &stones {
        // A cobble of the rock it lies on; on the limestone, half of them the granite the river
        // carried down from the hills.
        let (mesh, radius) = match &cobbles {
            Some((granite, limestone)) => {
                let [x, y] = stone.position;
                let on_limestone = geology.is_limestone(x, y, height.sample(x, y));
                let rock = if on_limestone && (stone.pick >> 16) & 1 == 1 {
                    limestone
                } else {
                    granite
                };
                (rock[stone.pick as usize % rock.len()], 1.0)
            }
            None => boulders[stone.pick as usize % boulders.len()],
        };
        builder.add_instance(
            mesh,
            Mat4::from_scale_rotation_translation(
                Vec3::splat(stone.radius as f32 / radius),
                Quat::from_rotation_y(stone.turn as f32 * std::f32::consts::TAU),
                Vec3::new(
                    stone.position[0] as f32 - half,
                    stone.bed as f32,
                    stone.position[1] as f32 - half,
                ),
            ),
        );
    }
    // The rocks: the GPU placement over the island's own heights, from its rock sites, the
    // granite's stones and the limestone's (`placement::RockRule::Sites`, #130); or with
    // `--no-rock-sites`, the city's boulders and rubble over its land (`RockRule::Land`). After
    // them, the movers' barrel (#79).
    let named = |names: &[(&str, StoneShape, [f32; 3])]| -> Vec<MeshId> {
        names
            .iter()
            .map(|(name, _, _)| ids[props.iter().position(|p| p.name == *name).expect("stone")])
            .collect()
    };
    let (rocks, second_rocks) = if sites.is_some() {
        (
            named(&ISLAND_STONES[..GRANITE_STONES]),
            named(&ISLAND_STONES[GRANITE_STONES..]),
        )
    } else {
        (
            ids[tiles + 1..ids.len() - usize::from(args.movers > 0)].to_vec(),
            Vec::new(),
        )
    };
    let meshes = CityMeshes {
        buildings: Vec::new(),
        rocks: rocks.clone(),
        second_rocks,
        // No city: none of these is placed.
        lamp: rocks[0],
        fountain: rocks[0],
        column: rocks[0],
    };
    let first = builder.reserve_instances(&placement::mesh_counts(&layout, &meshes));
    // The movers (#79), the table's last instances: their transforms come every frame.
    if args.movers > 0 {
        let barrel = *ids.last().expect("the barrel");
        builder.reserve_movers(&[(barrel, Barrels::movers(args.movers))]);
    }
    // No rock on the cells the channels are carved in or the lakes' shores smoothed (the
    // placement reads the 8 m samples, which those cells no longer follow), nor under a lake:
    // their samples are set far under the rocks' 3 m.
    let mut rock_ground = height.data.clone();
    let side = height.size - 1;
    for &c in channels.refined() {
        let (i, j) = (c % side, c / side);
        for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            rock_ground[((j + dj) * height.size + i + di) as usize] = -1.0e6;
        }
    }
    for lake in &lake_waters {
        for j in lake.first[1]..lake.first[1] + lake.size[1] {
            for i in lake.first[0]..lake.first[0] + lake.size[0] {
                if lake.covers(i, j) {
                    rock_ground[(j * height.size + i) as usize] = -1.0e6;
                }
            }
        }
    }
    // The rock sites (#130) clear of those samples: a cell with any of them at its corners
    // takes no rock.
    let site_cells = sites.as_ref().map(|(map, stats, ms)| {
        let mut cells = map.data.clone();
        let per = map.spacing / height.spacing;
        let last = height.size - 1;
        for (c, cell) in cells.iter_mut().enumerate() {
            let (x, y) = (c as u32 % map.size, c as u32 / map.size);
            let span = |k: u32| {
                let lo = (f64::from(k) * per).floor() as u32;
                lo.min(last)..=((f64::from(k + 1) * per).ceil() as u32).min(last)
            };
            let clear = span(y)
                .all(|j| span(x).all(|i| rock_ground[(j * height.size + i) as usize] > -1.0e5));
            if !clear {
                *cell = 0;
            }
        }
        let placed = layout.counts().rocks as f64;
        let total: f64 = stats.by_site.iter().sum();
        let by_site: Vec<String> = forge_procgen::SITE_NAMES
            .iter()
            .zip(stats.by_site)
            .map(|(name, w)| format!("{name} {:.0}", placed * w / total.max(1.0)))
            .collect();
        tracing::info!(
            rocks = layout.counts().rocks,
            granite = layout.first_rocks(),
            limestone = layout.counts().rocks - layout.first_rocks(),
            by_site = %by_site.join(", "),
            cells = cells.iter().filter(|&&c| c & 0x7f > 0).count(),
            ms,
            "the island's rock sites (#130, --no-rock-sites)"
        );
        (cells, map.size, map.spacing as f32)
    });
    // Drawn finer, the rocks stand on the drawn samples, those nearest a sample set aside above
    // set aside too (#106).
    let (rock_ground, rock_samples, rock_spacing) = if factor > 1 {
        let (n, coarse, f) = (drawn.size, height.size, factor);
        let fine: Vec<f32> = (0..n * n)
            .map(|s| {
                let (i, j) = (s % n, s / n);
                let nearest = ((j + f / 2) / f) * coarse + (i + f / 2) / f;
                if rock_ground[nearest as usize] < -1.0e5 {
                    -1.0e6
                } else {
                    drawn.heights[s as usize]
                }
            })
            .collect();
        (fine, n, drawn.spacing)
    } else {
        (rock_ground, height.size, height.spacing)
    };
    let ground = Ground {
        heights: &rock_ground,
        samples: rock_samples,
        spacing: rock_spacing as f32,
        sites: site_cells
            .as_ref()
            .map(|(cells, size, spacing)| placement::RockSites {
                cells,
                size: *size,
                spacing: *spacing,
            }),
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
    // The start view's pages, now the rocks are placed (#121).
    scene.load_start_view(&ctx.device)?;
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
fn build_city(
    ctx: &Context,
    args: &Args,
    cooked: Cooked,
    camera: &FlyCamera,
) -> Result<MeshletScene> {
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
    if streamed {
        set_start_view(&mut builder, ctx, args, layout.origin, camera);
    }
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
        sites: None,
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
        second_rocks: Vec::new(),
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
    // The start view's pages, now the instances are placed (#121).
    scene.load_start_view(&ctx.device)?;
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

/// The `city-blocks` binary: the city, `--gallery` or `--island SEED`.
pub fn main_city() -> Result<()> {
    run(Args::parse(), "forge city-blocks")
}

/// The `island` binary (#96's step 3): the same demo with the island of `--island` (seed 7
/// unless given) as its scene.
pub fn main_island() -> Result<()> {
    let matches = Args::command()
        .name("island")
        .about("The island: a 16 km island from a seed, its water, its rivers and its lakes")
        .get_matches();
    let mut args = Args::from_arg_matches(&matches)?;
    args.island.get_or_insert(ISLAND_SEED);
    run(args, "forge island")
}

/// The island the `island` binary draws unless `--island` picks another.
const ISLAND_SEED: u64 = 7;

/// Runs the demo over `args`, its window titled `title`.
fn run(args: Args, title: &'static str) -> Result<()> {
    let island = forge_procgen::RibbonParams::island();
    RIBBON_PARAMS
        .set(forge_procgen::RibbonParams {
            regional: island
                .regional
                .filter(|_| args.river_k > 0.0)
                .map(|(_, depth)| (args.river_k, depth)),
            steps: island.steps.filter(|_| !args.no_steps),
            brooks: island.brooks.filter(|_| !args.no_brooks),
            delta: island.delta.filter(|_| !args.no_deltas),
            bars: island.bars.filter(|_| !args.no_bars),
            ..island
        })
        .expect("the rivers' parameters, set once");
    SILLS.set(!args.no_sills).expect("the sills, set once");
    let config = AppConfig {
        title: title.into(),
        vsync: args.vsync,
        validate: args.validate,
        frame_limit: args.frames,
        capture: args.capture.clone().map(|p| (p, args.capture_frame)),
        capture_every: args.capture_every,
        overlay: if args.overlay { Some(true) } else { None },
        force_fallback: args.force_fallback,
        hdr: args.hdr,
        hdr_stops: args.hdr_stops,
        hdr_ui_white: args.hdr_ui_white,
        width: args.width,
        height: args.height,
        ..AppConfig::default()
    };
    // The props cook (or load from the cache) behind the loading screen (issue #25).
    forge_app::run_loading(config, move || {
        let cooked = cook(&args);
        let finish: Finish<Gallery> = Box::new(move |ctx| Gallery::new(ctx, args, cooked, title));
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

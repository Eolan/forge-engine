//! `city-blocks` — the Phase 1 closing demo (issue #13), built in steps. The twenty
//! procedural props of the city set (0.5 to 3 M triangles each, issue #34) and a 4 km
//! terrain (8 M triangles) are cooked into cluster DAGs once and cached on disk
//! (`mesh-cache/`); a compute pass places a million instances of the props over the terrain
//! (issue #35): a street grid of buildings, lamp posts and plazas, and rocks over the hills
//! around it. Their cluster pages stream from the cache files through a GPU pool as the LOD
//! cut asks for them (issue #36); `--fly` flies a loop at 300 m/s through TAA (issue #13).
//! `--gallery` shows the twenty props side by side instead; `--lab SCENE` one of `physics-lab`'s
//! scenes (issue #136, Space throws a ball, Enter starts it over).
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
use forge_app::{AppConfig, Context, Demo, Finish, FlyCamera, FrameInfo, HdrMode, Input, Setup};
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
    AmbientLight, Atmosphere, AtmosphereParams, AutoExposure, Bloom, CloudParams, Clouds,
    CullCamera, CullFlags, FrameStats, GroundSky, Gtao, GtaoParams, HdrOutput, LiquidDrawParams,
    LiquidStats, LiquidTank, LuminanceMeter, MeshletRenderer, MeshletScene, MeshletSceneBuilder,
    ProbeParams, Probes, Residency, SkyParams, SplashParams, SplashSource, Starfield, StartView,
    StreamingConfig, StreamingStats, SwRaster, Taa, Tonemap, WaterCascadeDesc, WaterCascades,
    WaterCaustics, WaterLake, WaterMouth, WaterPool, WaterRiverPoint, WaterShore, WaterShoreTrain,
    WaterSplashes, WaterStone, WaterSurface, WaterSurfaceParams, WaterWakes, exposure_from_ev100,
    sh_irradiance,
};
use forge_task::TaskPool;
use glam::{Mat4, Quat, Vec2, Vec3};
use winit::keyboard::KeyCode;

mod afloat;
mod island_demo;
mod island_sand;
mod island_walk;
mod lab;

use afloat::Barrels;

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
    /// The metered exposure's compensation in stops: positive brighter, negative darker (the
    /// sunlit parts of a view metered on its shade then clip less).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    exposure_compensation: f32,
    /// The fractions of the sorted pixels the exposure meters, `LOW,HIGH` (0.5,0.98 by default:
    /// the brighter half without the brightest highlights).
    #[arg(long, value_parser = parse_pair)]
    meter_band: Option<(f32, f32)>,
    /// Keep `--ev100` under `--day` and `--time-of-day` instead of metering the scene (frames
    /// compared at one exposure).
    #[arg(long)]
    fixed_exposure: bool,
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
    /// Supersample 2 × 2 (D-045, for screenshots): the frame drawn at twice the window's width
    /// and height and filtered down, about four times the cost.
    #[arg(long)]
    ssaa: bool,
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
    /// Leave the rivers' beds below their confluences as they were: no deeper scour hole
    /// (#119's polish).
    #[arg(long)]
    no_scour: bool,
    /// Lay no bar of sand along the bank past each confluence (#119's polish).
    #[arg(long)]
    no_confluence_bars: bool,
    /// No distributaries: the large rivers reach the sea in one channel (#127).
    #[arg(long)]
    no_distributaries: bool,
    /// Leave every beach of the island pale sand: no shingle on the headlands and under steep
    /// land (#128).
    #[arg(long)]
    no_beach_types: bool,
    /// Let the grass grow down to the salt water: no sand on the banks beside the sea and the
    /// rivers' tidal reaches over the beach's own top (#199).
    #[arg(long)]
    no_salt: bool,
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
    /// Show one of `physics-lab`'s scenes instead of the city (issue #136): rigid bodies on a
    /// flat floor through `forge-physics`.
    #[arg(long, value_enum)]
    lab: Option<lab::LabScene>,
    /// `--lab models`: show this model alone (its name as `tools/fetch-assets.sh` lists it, e.g.
    /// `WaterBottle`), framed for a capture beside Khronos's screenshot (#170, D-048).
    #[arg(long)]
    model: Option<String>,
    /// With `--lab`, write the session's commands and digests to this file at exit (#137).
    #[arg(long)]
    record: Option<PathBuf>,
    /// With `--lab`, play a recorded session again instead of the keys, checking its digests.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// With `--lab`, run the scene through a server and this player's client over a link of
    /// this many milliseconds one way, 2 % of the packets lost, a bot throwing too (#137).
    #[arg(long)]
    net: Option<f64>,
    /// With `--lab`, throw a ball from the camera every this many frames, as Space does.
    #[arg(long)]
    throw_every: Option<u64>,
    /// With `--lab sea`, the boat's throttle and rudder from the first frame, `T,R` (−1 to 1),
    /// in place of the arrow keys (#138).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    steer: Option<Vec<f32>>,
    /// With `--lab walk` or `--walker`, the player's walk from the first frame, `X,Z` in m/s
    /// along the ground, in place of the keys (#139, #196).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    walk: Option<Vec<f32>>,
    /// With `--island`, the walker on the ground at `X,Z` (the scene's metres) from the first
    /// frame, as Enter puts it under the camera (#196); bare, on the southern beach the camera
    /// starts off. (Negative coordinates as `--walker=-120,40`.)
    #[arg(long, value_delimiter = ',', num_args = 0..)]
    walker: Option<Vec<f32>>,
    /// With `--island`: the walker without the sand round it (#197), the tiles drawing the ground
    /// there, for an A/B of the window's untouched ground against theirs.
    #[arg(long)]
    no_sand_window: bool,
    /// With `--lab fly`, the aeroplane's controls from the first frame, `T,E,A,R` (throttle
    /// 0 to 1, elevator, ailerons and rudder −1 to 1), in place of the keys (#141).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    pilot: Option<Vec<f32>>,
    /// The cloud layer's share of the sky (#145; 0 to 1): fair-weather cumulus by default (the
    /// owner's choice, 2026-10-03), 0 for none.
    #[arg(long, default_value_t = 0.45)]
    clouds: f32,
    /// With `--lab creatures`, let the creatures' motors go at this frame, as ↓ does (#143).
    #[arg(long)]
    limp_at: Option<u64>,
    /// At this frame, as Space does: the wrecking ball let go (`--lab break`, #142), the flood's
    /// gate lifted (#144), the first domino tipped (#146), the convoy sent across (#147). The
    /// glass tank's gate lifted (#156).
    #[arg(long)]
    release: Option<u64>,
    /// `--lab tank` (#156): the liquid's pressure sweeps a substep (red and black each), without
    /// `--liquid-cycles`.
    #[arg(long, default_value_t = 32)]
    liquid_sweeps: u32,
    /// `--lab tank`: the liquid's cell, metres (590 000 particles at 1.25 cm, the default; 1.15
    /// million at 1 cm).
    #[arg(long, default_value_t = lab::tank::CELL)]
    liquid_cell: f32,
    /// `--lab tank`: the liquid's gravity, m/s² (x,y,z; `0,0,0` for none). Without surface
    /// tension, none leaves the water as it stands.
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true, default_values_t = [0.0, -9.81, 0.0])]
    liquid_gravity: Vec<f32>,
    /// `--lab tank`: what the tank's drawing shows: the water as it looks (key 1), its speed view
    /// (the surface coloured by the flow's speed; key 2) or its landing view (where its bent rays
    /// land; key 3).
    #[arg(long, value_enum, default_value_t = LiquidView::Look)]
    liquid_view: LiquidView,
    /// `--lab tank`: how long the air the water takes in lasts, seconds (fresh water's 0.3 by
    /// default: white only where a jet plunges; longer for sea water's foam; 0 for none).
    #[arg(long, default_value_t = forge_render::FRESH_FOAM_LIFE)]
    liquid_foam: f32,
    /// `--lab tank`: the share of the particles' crowding undone a substep.
    #[arg(long, default_value_t = 0.25)]
    liquid_drift: f32,
    /// `--lab tank`: the pressure sweeps' over-relaxation.
    #[arg(long, default_value_t = 1.7)]
    liquid_omega: f32,
    /// `--lab tank`: multigrid V-cycles of the liquid's pressure a substep, in place of the
    /// sweeps (0: the sweeps).
    #[arg(long, default_value_t = 0)]
    liquid_cycles: u32,
    /// `--lab tank`: with `--liquid-cycles`, the red-black sweeps before and after each level's
    /// correction.
    #[arg(long, default_value_t = 2)]
    liquid_smooth: u32,
    /// `--lab tank`: their over-relaxation.
    #[arg(long, default_value_t = 1.0)]
    liquid_smooth_omega: f32,
    /// `--lab tank`: the lab's ticks between the liquid's lines in the log.
    #[arg(long, default_value_t = 60)]
    liquid_log: u64,
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
    /// With `--fixed-step`, the rate the day, the exposure and the water advance at, steps a
    /// second (60): a high rate reproduces what a fast frame rate does in a scripted run.
    #[arg(long, default_value_t = 60.0)]
    step_hz: f32,
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
    /// Rays per probe and frame (64 to 256): 128, and 256 in the models lab's rooms,
    /// whose probes, lit through a few openings and seen at an indoor exposure, shimmered with
    /// 128 (#171: 7.5 % of a still view's pixels changed over the jitter's cycle, 2.6 % with
    /// 256, as many as with no probes at all).
    #[arg(long)]
    probe_rays: Option<u32>,
    /// Probe cascades, 4 m apart for the finest and twice as far each after (1 to 6).
    #[arg(long, default_value_t = ProbeParams::default().cascades)]
    probe_cascades: u32,
    /// The finest probe cascade's spacing in metres, the next ones twice as far each: 4, and
    /// `ROOM_PROBE_SPACING` in the models lab's rooms.
    #[arg(long)]
    probe_spacing: Option<f32>,
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
    /// The sea's water, by name (#108's sheet, `SeaWater::NAMED`): teal (the default), clear,
    /// blue, clear-blue, turquoise or ocean.
    #[arg(long, default_value = "teal", value_parser = parse_sea_water)]
    sea_water: forge_render::SeaWater,
    /// Moving geometry (#79): this many barrels drifting down the island's largest rivers,
    /// their transforms written every frame. None by default, so the reference captures stay
    /// put.
    #[arg(long, default_value_t = 0)]
    movers: u32,
    /// Draw the movers with the camera's motion vectors alone, not their own (#79's A/B: TAA
    /// then smears them).
    #[arg(long)]
    no_mover_motion: bool,
    /// Skin the lab's creatures and gulls by dual quaternions, not linear blending: a twisting
    /// joint keeps its thickness, a bending one swells a little (#169, D-052; on request only).
    #[arg(long)]
    dual_quaternion: bool,
    /// `--lab creatures`: draw the mannequins' left forearm twisted or bent 90° on top of their
    /// pose, to compare the skinning's blends (#169).
    #[arg(long, value_enum)]
    arm_pose: Option<lab::ArmPose>,
    /// Leave the rivers' flow undisturbed by the movers (#107's A/B).
    #[arg(long)]
    no_floaters: bool,
    /// No waves from the movers in the lakes and the sea (#107's A/B for the wakes).
    #[arg(long)]
    no_wakes: bool,
    /// Light the scene by the clear sky even under clouds (#163's A/B).
    #[arg(long)]
    no_cloud_light: bool,
    /// Draw the flood's columns themselves, not the GPU's finer layer that shadows them (#162's
    /// A/B).
    #[arg(long)]
    no_gpu_water: bool,
    /// The GPU's layer: fine cells along a column's side (#162).
    #[arg(long, default_value_t = 4)]
    gpu_water_ratio: u32,
    /// The GPU's layer: its steps a tick of the columns'.
    #[arg(long, default_value_t = 2)]
    gpu_water_substeps: u32,
    /// The GPU's layer: the share of its gap to the columns it closes a frame.
    #[arg(long, default_value_t = 0.25)]
    gpu_water_rate: f32,
    /// No spray where the water splashes: the steps' falls, a barrel dropped into a lake, the
    /// towed barrel's bow (#107's A/B for the splashes).
    #[arg(long)]
    no_splashes: bool,
    /// Draw the spray without the reactive mask, so TAA keeps its history there (the mask's
    /// A/B, #107).
    #[arg(long, hide = true)]
    no_reactive: bool,
    /// No rings where the splashes' drops land on still water (#107's A/B for them).
    #[arg(long)]
    no_drop_rings: bool,
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
    /// Leave each frame's mirror ray to TAA alone, without the reflections' history (#176,
    /// D-050): to compare with.
    #[arg(long)]
    no_reflection_history: bool,
    /// Hard sun shadows: one ray to the sun's centre instead of its disc (Z toggles them).
    #[arg(long)]
    hard_shadows: bool,
    /// The sun's apparent radius for the soft shadows, in degrees (D-049). Where a denoiser
    /// smooths them, 1° by default: softer than the real sun's 0.27°, for art's sake. Without
    /// one, the real sun's. The Moon's shadows at night scale with it.
    #[arg(long)]
    sun_size: Option<f32>,
    /// The denoiser of the sun's soft shadows (D-049): NVIDIA's SIGMA, where NRD is in
    /// `nrd-sdk/bin` (#172), else AMD's FidelityFX (#173), the default without NRD. F6 steps
    /// through SIGMA, FFX and none at run time.
    #[arg(long, value_enum)]
    shadow_denoiser: Option<ShadowDenoiserArg>,
    /// Keep the resolve's own shadow ray, no denoiser (#172): the soft shadows as before, at the
    /// real sun's size unless `--sun-size` says otherwise.
    #[arg(long)]
    no_shadow_denoiser: bool,
    /// The sun's shadow from 256 rays a pixel and frame, without a denoiser: the reference the
    /// denoisers are judged by (#172), at their sun size. Slow: a development view.
    #[arg(long)]
    shadow_reference: bool,
    /// A day over the city: the sun rises in the east, crosses the south and sets in the west in
    /// this many seconds, and the exposure follows it (issue #57); then the night as long, under
    /// the Moon and the stars (D-046, issue #164), and again.
    #[arg(long)]
    day: Option<f32>,
    /// The sun held where `--day` has it this far through the cycle (0 sunrise, 0.5 noon, 1
    /// sunset, 1.5 midnight), the exposure metered from the scene as `--day`'s.
    #[arg(long, conflicts_with = "day")]
    time_of_day: Option<f32>,
    /// The Moon's age with `--day` and `--time-of-day`, in lunations: 0 new, 0.25 first
    /// quarter, 0.5 full (D-046; default 0.4, a waxing gibbous).
    #[arg(long)]
    moon_age: Option<f32>,
    /// The Moon's light as a soft unshadowed fill in the sky's light, instead of a key light
    /// with traced shadows like the sun's (D-046's other answer, to compare).
    #[arg(long)]
    moon_fill: bool,
    /// How much brighter than their physical luminance the night's stars and Milky Way are
    /// drawn (default 16: as a dark-adapted eye sees them rather than a camera).
    #[arg(long)]
    star_gain: Option<f32>,
    /// How many stops under what the eye would adapt to the night is shown (D-046, film's
    /// night): 2 by default, which leaves the moonlit land about 4 stops under its day's
    /// brightness, the scene metered on its brighter half (the sky and the clouds).
    #[arg(long, default_value_t = 2.0)]
    night_stops: f32,
    /// No Purkinje shift at night (the colours kept as the cones see them), to compare.
    #[arg(long)]
    no_purkinje: bool,
    /// The real sky at night (D-046): the Yale Bright Star Catalogue's stars and the Moon's
    /// albedo map, in place of the procedural stars and a plain Moon. `.` switches it.
    #[arg(long)]
    real_sky: bool,
    /// The camera's vertical field of view, degrees (70 by default): narrow for a telephoto
    /// look at the Moon.
    #[arg(long)]
    fov: Option<f32>,
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
    /// Slide sideways from the start view at this many metres per second, to the right (a frame at a
    /// time with `--fixed-step`): every edge moves across the screen and TAA resamples its history
    /// each frame, for measuring sharpness in motion (#159).
    #[arg(long, conflicts_with_all = ["fly", "orbit", "tour", "dolly"])]
    pan: Option<f32>,
    /// Bloom strength, the share of the shown image that is bloom (0 for none; B toggles it).
    #[arg(long, default_value_t = 0.04)]
    bloom: f32,
    /// Draw without TAA (no jitter, no history): the raw frame, aliased.
    #[arg(long)]
    no_taa: bool,
    /// How strongly TAA's image is sharpened (FidelityFX RCAS, D-045): this many stops below
    /// its strongest, each stop half as strong. T cycles TAA sharpened, plain and off.
    #[arg(long, default_value_t = 0.5)]
    rcas: f32,
    /// Show TAA's image unsharpened.
    #[arg(long)]
    no_rcas: bool,
    /// What the props' textures keep: full (colour and relief), flat (colour alone) or none
    /// (plain colours), to compare.
    #[arg(long, value_enum, default_value_t = TextureMode::Full)]
    textures: TextureMode,
    /// Let the tank's water bend the scene as drawn, aliased, rather than anti-aliased by a TAA
    /// of its own (#156: the A/B of that TAA).
    #[arg(long)]
    no_behind_taa: bool,
    /// Resample TAA's history through Catmull-Rom (5 bilinear fetches), as before D-045, rather
    /// than Lanczos-3 (36 texels).
    #[arg(long)]
    taa_catmull_rom: bool,
    /// Anti-alias with NVIDIA's DLAA (DLSS at the window's own resolution) in place of TAA in a
    /// scripted run too (`--frames`). DLAA is the default of an interactive run where it runs
    /// (D-045: the Streamline SDK in `streamline-sdk/` and an RTX GPU); a scripted run keeps to
    /// TAA, whose images repeat to the bit.
    #[arg(long, conflicts_with_all = ["no_taa", "no_dlaa"])]
    dlaa: bool,
    /// Anti-alias with TAA even where DLAA runs.
    #[arg(long)]
    no_dlaa: bool,
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
    /// The cloud layer (#145, `--clouds`), and last frame's view-projection it reprojects by.
    clouds: Option<Clouds>,
    clouds_previous: Mat4,
    /// In `--lab space`, the sky of space in place of the ground's.
    space: Option<SpaceSky>,
    /// In `--lab tank`, the GPU liquid (#156), and per frame slot the lab's tick its statistics
    /// were asked at.
    liquid: Option<forge_render::Liquid>,
    /// The TAA of the scene behind the tank's water (#156), its own history.
    behind_taa: Option<Taa>,
    /// `FORGE_HASH_IMAGES=1`: per-frame image hashes in the log (#161).
    hasher: Option<forge_render::debug_hash::ImageHasher>,
    /// The GPU's finer layer over the flood's columns (#162), made at the first frame that has
    /// them, and the columns' tick it last followed.
    shallow: Option<forge_render::ShallowLayer>,
    shallow_tick: Option<u64>,
    liquid_asked: [Option<u64>; forge_gpu::FRAMES_IN_FLIGHT],
    /// The lab's tick at which the liquid's next line is due.
    liquid_next_log: u64,
    /// Enter was pressed: the liquid starts over at the next frame.
    liquid_reset: bool,
    /// What the liquid's drawing shows: as it looks (1), its speed view (2), its landing view (3).
    liquid_mode: forge_render::LiquidMode,
    /// DLAA where it runs (D-045): DLSS at the output's size and the display pass it hands its
    /// HDR image to, in place of TAA while `dlaa_on` (T cycles it with TAA).
    dlaa: Option<(forge_render::DlssUpscaler, forge_render::Display)>,
    dlaa_on: bool,
    /// The denoisers of the sun's soft shadows (D-049): NVIDIA's SIGMA where NRD's library is in
    /// `nrd-sdk/bin` (#172), then AMD's FidelityFX (#173); `sun_shadow_on` the one on (F6), or
    /// none.
    sun_shadows: Vec<forge_render::SunShadowDenoiser>,
    sun_shadow_on: Option<usize>,
    /// The mirror rays' history (#176, D-050): the glass's reflections settled before TAA.
    reflection_history: forge_render::ReflectionHistory,
    /// The shadows' sun over the real one's size (D-049: softer where a denoiser smooths them,
    /// `--sun-size`); the Moon's at night scales with it.
    sun_scale: f32,
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
    /// The island's walker, while one walks (#196): Enter puts it on the ground under the camera.
    walker: Option<island_walk::Walker>,
    /// The island's sand round the walker (#197).
    sand: Option<island_sand::SandWindow>,
    /// `--lab` (#136): the physics lab's world, whose bodies are the movers.
    /// The camera follows the lab's boat (C), and the throttle and rudder last sent (#138).
    chase: bool,
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
    lab: Option<lab::Lab>,
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
    /// The night (D-046, issue #164) with `--day` and `--time-of-day`: where the Moon and the
    /// stars are, the stars on the GPU, and this frame's sky and lights.
    night: Option<forge_render::night::NightSky>,
    celestial: forge_render::night::Celestial,
    night_settings: forge_render::night::NightSettings,
    night_now: Option<(forge_render::night::SkyAt, forge_render::night::NightLights)>,
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

/// `--lab space`'s sun: from the ship's right and a little behind it and above, so the chase
/// camera sees its lit side and the planet's day side with its terminator far to the left.
const SPACE_SUN: Vec3 = Vec3::new(0.75, 0.35, 0.5);
/// The planet: its direction from the scene, low on the left ahead of the ship, and its angular
/// radius (32°, the Earth's from about 5 600 km up), so its face shows its oceans, land and
/// clouds rather than only the haze along its limb (from low orbit, 70°, it was a grey wall).
const PLANET_DIR: Vec3 = Vec3::new(-0.55, -0.45, -0.7);
const PLANET_ANGLE_DEG: f32 = 32.0;
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

/// `--shadow-denoiser`: which denoiser smooths the sun's soft shadows (D-049).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum ShadowDenoiserArg {
    /// NVIDIA's SIGMA (NRD, #172).
    Sigma,
    /// AMD's FidelityFX shadow denoiser (#173).
    Ffx,
}

impl ShadowDenoiserArg {
    fn kind(self) -> forge_render::ShadowDenoiserKind {
        match self {
            Self::Sigma => forge_render::ShadowDenoiserKind::Sigma,
            Self::Ffx => forge_render::ShadowDenoiserKind::Ffx,
        }
    }
}

/// `--liquid-view`: what the tank's drawing shows (#156).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum LiquidView {
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

/// The sky of space (`--lab space`, the owner's ask of 2026-10-03): the asteroids' starfield and
/// the sun's disc, and an Earth-like planet under its atmosphere seen from orbit (D-023's
/// models), drawn where the geometry left the depth clear, in place of the ground's sky. Its shaded
/// sides take space's constant fill (no sky's light, no probes), occluded by GTAO.
struct SpaceSky {
    starfield: Starfield,
    /// The planet's air, and the camera from its centre (km).
    planet: Atmosphere,
    view: Vec3,
}

impl SpaceSky {
    fn new(ctx: &Setup, sun_illuminance: f32) -> Result<Self> {
        let params = AtmosphereParams::earth();
        let view = params.view_from_space(PLANET_DIR, PLANET_ANGLE_DEG.to_radians());
        let mut planet = Atmosphere::new(&ctx.device, &ctx.shaders, params)?;
        planet.planet_view = true;
        let mut starfield = Starfield::faint(&ctx.device, &ctx.shaders, forge_render::HDR_FORMAT)?;
        starfield.sun_illuminance = sun_illuminance;
        Ok(Self {
            starfield,
            planet,
            view,
        })
    }
}

/// Metres between the centres of neighbouring props in the gallery.
const SPACING: f32 = 60.0;
/// Props per row of the gallery.
const COLUMNS: u32 = 5;

impl Gallery {
    fn new(ctx: &Setup, args: Args, cooked: Cooked, title: &'static str) -> Result<Self> {
        let args = island_demo::with_shot(args)?;
        let mut renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, ctx.extent())?;
        let mut taa = Taa::new(&ctx.device, &ctx.shaders, ctx.extent(), ctx.output.format)?;
        taa.enabled = !args.no_taa;
        taa.bloom_strength = args.bloom;
        taa.sharpen = (!args.no_rcas).then_some(args.rcas);
        taa.lanczos = !args.taa_catmull_rom;
        let bloom = Bloom::new(&ctx.device, &ctx.shaders)?;
        let bloom_on = args.bloom > 0.0;
        let sky_light = !args.no_sky_light;
        let gtao = Gtao::new(&ctx.device, &ctx.shaders)?;
        let meter = LuminanceMeter::new(&ctx.device, &ctx.shaders)?;
        let mut auto_exposure = AutoExposure::new(args.ev100);
        auto_exposure.compensation = args.exposure_compensation;
        if let Some(band) = args.meter_band {
            auto_exposure.band = band;
        }
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
        // The sun's soft shadows denoised (D-049): by NVIDIA's SIGMA where NRD's library is there
        // (#172), and by AMD's FidelityFX everywhere (#173), the first of them on.
        let mut sun_shadows = Vec::new();
        if !args.no_shadow_denoiser && !args.shadow_reference {
            let dir = std::env::var_os("FORGE_NRD_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR")).join("nrd-sdk/bin")
                });
            match forge_render::SunShadowDenoiser::sigma(&ctx.device, &dir, ctx.extent()) {
                Ok(denoiser) => sun_shadows.push(denoiser),
                Err(error) => tracing::info!(%error, "no NRD: no SIGMA for the sun's shadows"),
            }
            sun_shadows.push(forge_render::SunShadowDenoiser::ffx(
                &ctx.device,
                &ctx.shaders,
                ctx.extent(),
            )?);
        }
        let reflection_history = forge_render::ReflectionHistory::new(&ctx.device, ctx.extent())?;
        let sun_shadow_on = match args.shadow_denoiser {
            _ if sun_shadows.is_empty() => None,
            None => Some(0),
            Some(wanted) => match sun_shadows.iter().position(|d| d.kind() == wanted.kind()) {
                Some(index) => Some(index),
                None => {
                    tracing::warn!(
                        ?wanted,
                        "that denoiser is not there (no NRD?): the other one"
                    );
                    Some(0)
                }
            },
        };
        if let Some(index) = sun_shadow_on {
            let denoiser = &sun_shadows[index];
            tracing::info!(denoiser = denoiser.kind().name(), version = %denoiser.version(), "the sun's shadows denoised (F6 steps through SIGMA, FFX and none)");
        }
        // The shadows' sun: the real one, or softer where a denoiser smooths them (D-049).
        let real = forge_render::starfield::SUN_ANGULAR_RADIUS_1AU;
        let sun_scale = match args.sun_size {
            Some(degrees) => degrees.to_radians() / real,
            None if sun_shadow_on.is_some() || args.shadow_reference => {
                forge_render::DENOISED_SUN_RADIUS / real
            }
            None => 1.0,
        };
        // The sun's disc softens the shadows (issue #54).
        if !args.hard_shadows {
            renderer.sun_angular_radius = real * sun_scale;
        }
        let atmosphere = Atmosphere::new(&ctx.device, &ctx.shaders, atmosphere_params)?;
        let sky = GroundSky::new(&ctx.device, &ctx.shaders)?;
        // In space (the owner's ask of 2026-10-03): the sun unfiltered by any air, from the right
        // and behind the ship; stars, its disc and a planet below for the sky.
        let space = (args.lab == Some(lab::LabScene::Space))
            .then(|| SpaceSky::new(ctx, renderer.sun_illuminance))
            .transpose()?;
        if space.is_some() {
            renderer.sun_dir = SPACE_SUN.normalize();
            renderer.sun_color = Vec3::ONE;
        }
        // The glass tank's liquid (#156): pure water, the solver as the arguments set it; on its
        // bench, tinted, under a white sun from the left and behind.
        let bench = args.lab == Some(lab::LabScene::TankBench);
        let liquid_mode = args.liquid_view.mode();
        if bench {
            renderer.sun_dir = BENCH_SUN.normalize();
            renderer.sun_color = Vec3::ONE;
        }
        // The sharpness room (#159): the sun alone, white, from the front left.
        let room = args.lab == Some(lab::LabScene::Room);
        if room {
            renderer.sun_dir = lab::room::SUN.normalize();
            renderer.sun_color = Vec3::ONE;
        }
        // The yard (#185): a low sun across the beds, the prints in raking light.
        if args.lab == Some(lab::LabScene::Yard) {
            renderer.sun_dir = lab::yard::SUN.normalize();
        }
        // The materials' patches (#205): the yard's raking sun, for the prints too.
        if args.lab == Some(lab::LabScene::Materials) {
            renderer.sun_dir = lab::yard::SUN.normalize();
        }
        // The night under the ground's sky (D-046) wherever the day turns: `--day`, `--time-of-day`.
        let night = ((args.day.is_some() || args.time_of_day.is_some())
            && space.is_none()
            && !bench
            && !room)
            .then(|| {
                let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
                let mut night = forge_render::night::NightSky::new(
                    &ctx.device,
                    renderer.sun_illuminance,
                    Some(&root),
                )?;
                night.show_real = args.real_sky;
                Ok::<_, forge_gpu::GpuError>(night)
            })
            .transpose()?;
        let mut celestial = forge_render::night::Celestial::default();
        if let Some(age) = args.moon_age {
            celestial.moon_age = age;
        }
        let night_settings = forge_render::night::NightSettings {
            moon_fill: args.moon_fill,
            star_gain: args
                .star_gain
                .unwrap_or(forge_render::night::NightSettings::default().star_gain),
            ..Default::default()
        };
        if night.is_some() {
            // Film's night (D-046): a few stops under what the eye would adapt to.
            auto_exposure.night_stops = args.night_stops;
        }
        let liquid = matches!(
            args.lab,
            Some(
                lab::LabScene::Tank
                    | lab::LabScene::TankBench
                    | lab::LabScene::TankHole
                    | lab::LabScene::TankBlocks
            )
        )
        .then(|| {
            forge_render::Liquid::new(
                &ctx.device,
                &ctx.shaders,
                lab::tank::liquid(
                    args.liquid_cell,
                    args.lab == Some(lab::LabScene::TankHole),
                    args.lab == Some(lab::LabScene::TankBlocks),
                ),
                forge_render::LiquidSolver {
                    sweeps: args.liquid_sweeps,
                    omega: args.liquid_omega,
                    cycles: args.liquid_cycles,
                    smooth: args.liquid_smooth,
                    smooth_omega: args.liquid_smooth_omega,
                    drift: args.liquid_drift,
                    gravity: Vec3::from_slice(
                        &[args.liquid_gravity.as_slice(), &[0.0; 3]].concat(),
                    ),
                    ..forge_render::LiquidSolver::default()
                },
                forge_render::LiquidLook {
                    foam_life: args.liquid_foam,
                    ..if bench {
                        forge_render::LiquidLook::tinted()
                    } else {
                        forge_render::LiquidLook::pure_water()
                    }
                },
            )
        })
        .transpose()?;
        // The scene behind the tank's water, anti-aliased by a TAA of its own before the water
        // bends it (#156): bent differently every frame, an aliased scene's edges cannot be
        // averaged after.
        let behind_taa = (liquid.is_some() && !args.no_behind_taa)
            .then(|| -> Result<Taa> {
                let mut behind =
                    Taa::new(&ctx.device, &ctx.shaders, ctx.extent(), ctx.output.format)?;
                behind.lanczos = taa.lanczos;
                Ok(behind)
            })
            .transpose()?;
        // The cloud layer (#145), over the given share of the sky; none at 0, nor in space, nor on the
        // tank's bench, nor in the sharpness room.
        let clouds = (args.clouds > 0.0 && space.is_none() && !bench && !room)
            .then(|| Clouds::new(&ctx.device, &ctx.shaders, ctx.extent()))
            .transpose()?;
        // DLAA in place of TAA (D-045) where the device has DLSS.
        let dlaa = if wants_dlaa(&args) {
            let dlss = forge_render::DlssUpscaler::new(
                &ctx.device,
                forge_render::DlssMode::Dlaa,
                ctx.extent(),
            )?;
            if dlss.is_none() && args.dlaa {
                tracing::warn!(
                    "DLAA is not available (it needs --features dlss, the Streamline SDK in streamline-sdk/ and an RTX GPU): TAA instead"
                );
            }
            dlss.map(|dlss| -> Result<_> {
                let display =
                    forge_render::Display::new(&ctx.device, &ctx.shaders, ctx.output.format)?;
                Ok((dlss, display))
            })
            .transpose()?
        } else {
            None
        };
        // Known before the scene, whose streamed pages it loads first (#121).
        let mut camera = start_camera(&args)?;
        if let Some(fov) = args.fov {
            camera.fov_y = fov.clamp(0.5, 120.0).to_radians();
        }
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
        let mut lab = None;
        let mut sand = None;
        let (scene, placed) = if let Some(kind) = args.lab {
            let (scene, mut built) = lab::build(ctx, &args, cooked, kind)?;
            built.arm_pose = args.arm_pose;
            lab = Some(built);
            (scene, Vec::new())
        } else if args.gallery {
            build_gallery(ctx, &args, cooked)?
        } else if args.island.is_some() {
            let (scene, window) = build_island(ctx, &args, cooked, &camera)?;
            sand = Some(window);
            (scene, Vec::new())
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
        if args.shadow_reference {
            flags.0 |= CullFlags::SHADOW_REFERENCE;
        }
        // The probes trace the scene's TLAS (issue #53).
        let probes_on = !args.no_probes;
        let probe_rays = args.probe_rays.unwrap_or(
            if args.lab == Some(lab::LabScene::Models) && lab::models::room_shown() {
                256
            } else {
                ProbeParams::default().rays
            },
        );
        anyhow::ensure!(
            (64..=256).contains(&probe_rays),
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
                rays: probe_rays,
                cascades: args.probe_cascades,
                cadence: args.probe_cadence,
                spacing: args.probe_spacing.unwrap_or(
                    if args.lab == Some(lab::LabScene::Models) && lab::models::room_shown() {
                        ROOM_PROBE_SPACING
                    } else {
                        ProbeParams::default().spacing
                    },
                ),
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
        let water = if args.lab == Some(lab::LabScene::Sea) && args.water() {
            // The physics lab's sea (#138): the island's waves, open, no shore.
            Some(lab::water(ctx)?)
        } else if args.lab == Some(lab::LabScene::Flood) && args.water() {
            // The flood's water (#144): a pool drawn from its columns, no sea; the cascades
            // still carry its ripples.
            let water = lab::water(ctx)?;
            water.1.set_sea(false);
            Some(water)
        } else if args.island.is_some() && args.water() {
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
        // The movers (#79): barrels afloat on the island's largest rivers (#177).
        let barrels = (args.movers > 0 && args.island.is_some())
            .then(|| -> Result<Barrels> {
                let start = Instant::now();
                let heights = island_heights(&args);
                let IslandRivers { rivers, lakes, .. } = island_ribbons(&heights);
                let barrels = Barrels::new(&heights, rivers, lakes, args.movers)?;
                // Views with the fixed step: of the first barrel at frame 60 (a second in) from 4 m
                // to its side and 1.5 m over it; of the first moored barrel the water runs past at
                // 1.2 m/s or more, from 6 m to its side and 7 m over it, looking a little
                // downstream; and of the towed barrel at frame 300, its wake grown, from 10 m
                // inside its circle and 8 m over it, looking back along its wake (#107).
                let view = barrels.view_of(0, 1.0, 4.0, 1.5, 0.0);
                // And of the first log and the first crate (#177), the same way.
                let log = if args.movers > 4 {
                    barrels.view_of(3, 1.0, 5.0, 2.0, 0.0)
                } else {
                    String::from("none")
                };
                let crate_ = if args.movers > 7 {
                    barrels.view_of(6, 1.0, 4.0, 1.5, 0.0)
                } else {
                    String::from("none")
                };
                let moored = barrels.moored_in(1.2).map_or(String::from("none"), |k| {
                    barrels.view_of(k, 0.0, 6.0, 7.0, 1.5)
                });
                let towed = if barrels.towing() {
                    barrels.view_of(args.movers - 1, 5.0, 10.0, 8.0, -4.0)
                } else {
                    String::from("none")
                };
                // The dropped barrel (#107's splashes) from 6 m off and 1.5 m over the water,
                // looking at where it meets it; with the fixed step it first does 2.73 s in, at
                // frame 164.
                let dropped = barrels
                    .drop_at()
                    .map_or(String::from("none"), |(at, level)| {
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
                    rivers = barrels.rivers(),
                    tiles = barrels.tiles,
                    ms = start.elapsed().as_millis(),
                    %view,
                    %log,
                    crate_view = %crate_,
                    %moored,
                    %towed,
                    %dropped,
                    tow_radius_m = barrels.tow_radius(),
                    "barrels afloat on the rivers (--movers)"
                );
                Ok(barrels)
            })
            .transpose()?;
        let wakes = (barrels.is_some() && water.is_some() && !args.no_wakes)
            .then(|| WaterWakes::new(&ctx.device, &ctx.shaders))
            .transpose()?;
        // `--walker X,Z` (#196): the island's walker on the ground there from the first frame.
        let walker = match &args.walker {
            Some(at) if args.lab.is_none() && args.island.is_some() => {
                let at = match at.as_slice() {
                    [] => Vec3::new(0.0, 0.0, island_beach(&args) as f32),
                    [x, rest @ ..] => Vec3::new(*x, 0.0, rest.first().copied().unwrap_or(0.0)),
                };
                let layers = sand.as_ref().map(island_sand::SandWindow::layers);
                Some(island_walk::Walker::new(island_drawn(&args), at, layers)?)
            }
            _ => None,
        };
        // With the water, or the yard's spray (#192).
        let splashes = ((water.is_some() || args.lab == Some(lab::LabScene::Yard))
            && !args.no_splashes)
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
            walker,
            sand,
            lab,
            // The car is followed from the start (C lets it go); the aeroplane always is.
            chase: matches!(args.lab, Some(lab::LabScene::Drive | lab::LabScene::Flyer)),
            steering: (0.0, 0.0),
            walking: [0.0; 2],
            handbrake: false,
            beds_seen: u64::MAX,
            beds_again: false,
            flying: [0.0; 4],
            pulling: 0.5,
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
            clouds,
            clouds_previous: Mat4::IDENTITY,
            space,
            liquid,
            behind_taa,
            shallow: None,
            shallow_tick: None,
            hasher: std::env::var_os("FORGE_HASH_IMAGES")
                .is_some_and(|v| v != "0")
                .then(|| forge_render::debug_hash::ImageHasher::new(&ctx.device, &ctx.shaders))
                .transpose()?,
            liquid_asked: [None; forge_gpu::FRAMES_IN_FLIGHT],
            liquid_next_log: 0,
            liquid_reset: false,
            liquid_mode,
            dlaa_on: dlaa.is_some(),
            dlaa,
            sun_shadow_on,
            sun_shadows,
            reflection_history,
            sun_scale,
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
            night,
            celestial,
            night_settings,
            night_now: None,
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
            let last = if gallery.night.is_some() { 2.0 } else { 1.0 };
            gallery.set_sun_of_day(t.clamp(0.0, last));
        }
        Ok(gallery)
    }

    /// Whether the exposure follows the scene's metered light (`--day`, `--time-of-day`) rather
    /// than `--ev100`.
    fn metered(&self) -> bool {
        (self.args.day.is_some() || self.args.time_of_day.is_some()) && !self.args.fixed_exposure
    }

    /// `--day` (issue #57): the sun at `t` of the cycle (0 sunrise, 0.5 noon, 1 sunset, 1.5
    /// midnight). It rises from 4° below the eastern horizon to 70° in the south and sets in the
    /// west, and its colour is the sunlight through the air. With the night (D-046), the key
    /// light goes to the Moon once the sun is down: its direction, colour and illuminance take
    /// the sun's fields, which are then per unit of the frame's reference illuminance.
    fn set_sun_of_day(&mut self, t: f32) {
        let sun_illuminance = forge_render::starfield::SUN_ILLUMINANCE_1AU;
        let sky = self.celestial.at(t, sun_illuminance);
        let params = &self.atmosphere.params;
        let through = |direction: Vec3| {
            Vec3::from(params.transmittance(
                Vec3::new(0.0, params.bottom_radius + 0.05, 0.0),
                direction,
                64,
            ))
        };
        if self.night.is_none() {
            self.renderer.sun_dir = sky.sun;
            self.renderer.sun_color = through(sky.sun);
            return;
        }
        let lights = forge_render::night::NightLights::new(&sky, sun_illuminance);
        self.renderer.sun_dir = lights.key;
        self.renderer.sun_illuminance = lights.reference;
        // `--moon-fill`: no direct moonlight (the fill is in the sky's light instead).
        let direct = if self.night_settings.moon_fill && !lights.key_is_sun {
            0.0
        } else {
            lights.key_weight
        };
        self.renderer.sun_color = through(lights.key) * direct;
        // The disc's softness, unless Z made the shadows hard.
        if self.renderer.sun_angular_radius > 0.0 {
            self.renderer.sun_angular_radius = lights.key_radius * self.sun_scale;
        }
        self.night_now = Some((sky, lights));
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
        for denoiser in &mut self.sun_shadows {
            denoiser.resize(ctx.extent())?;
        }
        self.reflection_history.resize(ctx.extent())?;
        if let Some(behind) = &mut self.behind_taa {
            behind.resize(ctx.extent())?;
        }
        if let Some((dlss, _)) = &mut self.dlaa {
            dlss.configure(forge_render::DlssMode::Dlaa, ctx.extent())?;
        }
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
            // Where the camera is, as `--view` takes it: to start a run where something was seen.
            KeyCode::F9 => {
                let (p, c) = (self.camera.position, &self.camera);
                tracing::info!(
                    "the view: --view={:.2},{:.2},{:.2},{:.1},{:.1}",
                    p.x,
                    p.y,
                    p.z,
                    c.yaw.to_degrees(),
                    c.pitch.to_degrees()
                );
            }
            // The real sky (D-046): the catalogue's stars and the Moon's albedo, or not.
            KeyCode::Period => {
                if let Some(night) = self.night.as_mut().filter(|n| n.has_real()) {
                    night.show_real = !night.show_real;
                    tracing::info!(real = night.show_real, "the real sky");
                }
            }
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
                    forge_render::starfield::SUN_ANGULAR_RADIUS_1AU * self.sun_scale
                };
            }
            KeyCode::KeyJ => self.flags.toggle(CullFlags::SHADOWS),
            // NVIDIA's SIGMA over the sun's shadows, or the resolve's own ray (#172).
            // The sun's shadows' denoiser: SIGMA, FFX, none, then again (#172, #173).
            KeyCode::F6 if !self.sun_shadows.is_empty() => {
                self.sun_shadow_on = match self.sun_shadow_on {
                    None => Some(0),
                    Some(index) if index + 1 < self.sun_shadows.len() => Some(index + 1),
                    Some(_) => None,
                };
                let name = match self.sun_shadow_on {
                    Some(index) => {
                        let denoiser = &mut self.sun_shadows[index];
                        denoiser.reset_history();
                        denoiser.kind().name()
                    }
                    None => "none",
                };
                tracing::info!(denoiser = name, "the sun's shadows' denoiser");
            }
            // DLAA where it runs, TAA sharpened, plain, off (D-045).
            KeyCode::KeyT => {
                let dlaa = self.dlaa.is_some();
                (self.dlaa_on, self.taa.enabled, self.taa.sharpen) =
                    match (self.dlaa_on, self.taa.enabled, self.taa.sharpen) {
                        (true, ..) => (false, true, Some(self.args.rcas)),
                        (false, true, Some(_)) => (false, true, None),
                        (false, true, None) => (false, false, None),
                        (false, false, _) if dlaa => (true, true, Some(self.args.rcas)),
                        (false, false, _) => (false, true, Some(self.args.rcas)),
                    };
                tracing::info!(
                    anti_aliasing = if self.dlaa_on {
                        "DLAA"
                    } else if !self.taa.enabled {
                        "none"
                    } else if self.taa.sharpen.is_some() {
                        "TAA, sharpened"
                    } else {
                        "TAA"
                    }
                );
                self.taa.reset_history();
            }
            // Space: the player jumps in the playground (#139), the car's handbrake on the track
            // (#140, held: see `update`), what a scene holds back let go (the wrecking ball #142,
            // the flood's gate, the tank's gate or shutter #156: `held`); elsewhere it throws, as
            // X does. On the island, the walker jumps (#196).
            KeyCode::Space => {
                if let Some(lab) = &mut self.lab {
                    if lab.has_player() {
                        lab.jump();
                    } else if lab.held() {
                        lab.release();
                    } else if !lab.has_car() {
                        lab.throw(self.camera.position, self.camera.forward());
                    }
                } else if let Some(walker) = &mut self.walker {
                    walker.jump();
                }
            }
            KeyCode::KeyX => {
                if let Some(lab) = &mut self.lab {
                    lab.throw(self.camera.position, self.camera.forward());
                }
            }
            KeyCode::Enter => {
                if let Some(lab) = &mut self.lab {
                    lab.reset();
                    // The tank's water too: seeded again, the tick that puts the gate back skipped.
                    self.liquid_reset = true;
                } else if self.args.island.is_some() {
                    // The island's walker (#196): on the ground under the camera, or gone and the
                    // camera free again where it is.
                    self.walker = match self.walker.take() {
                        Some(_) => None,
                        None => {
                            let layers = self.sand.as_ref().map(island_sand::SandWindow::layers);
                            island_walk::Walker::new(
                                island_drawn(&self.args),
                                self.camera.position,
                                layers,
                            )
                            .inspect_err(|e| tracing::warn!("the walker: {e}"))
                            .ok()
                        }
                    };
                }
            }
            // The tank's water as it looks, coloured by its speed, or by where its rays land (#156).
            KeyCode::Digit1 => self.liquid_mode = forge_render::LiquidMode::Look,
            KeyCode::Digit2 => self.liquid_mode = forge_render::LiquidMode::Speed,
            KeyCode::Digit3 => self.liquid_mode = forge_render::LiquidMode::Landing,
            KeyCode::KeyC => self.chase = !self.chase,
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
        self.step = if self.args.fixed_step {
            1.0 / self.args.step_hz
        } else {
            dt
        };
        self.sea_time = match self.args.sea_time {
            Some(still) => still,
            None => self.sea_time + f64::from(self.step),
        };
        if let Some(lab) = &mut self.lab {
            if let Some(every) = self.args.throw_every
                && self.frame > 0
                && self.frame.is_multiple_of(every.max(1))
            {
                lab.throw(self.camera.position, self.camera.forward());
            }
            if self.args.release == Some(self.frame) {
                lab.release();
            }
            // The creatures (#143): ↓ lets their motors go, ↑ powers them again; or
            // `--limp-at N`.
            if let Some(limp) = lab.creatures() {
                let wanted =
                    if self.args.limp_at == Some(self.frame) || input.is_down(KeyCode::ArrowDown) {
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
            if let Some(s) = &self.args.steer {
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
                let throttle = (self.flying[0]
                    + 0.5 * self.step * keys(KeyCode::KeyW, KeyCode::KeyS))
                .clamp(0.0, 1.0);
                let shift = input.is_down(KeyCode::ShiftLeft) || input.is_down(KeyCode::ShiftRight);
                let elevator = if shift { 1.0 } else { 0.5 };
                let mut flying = [
                    throttle,
                    elevator * keys(KeyCode::ArrowUp, KeyCode::ArrowDown),
                    keys(KeyCode::ArrowRight, KeyCode::ArrowLeft),
                    keys(KeyCode::KeyD, KeyCode::KeyA),
                ];
                if let Some(f) = &self.args.pilot {
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
            // The playground's player (#139): a command when the walk changes.
            if lab.has_player() {
                let walk = wished_walk(&self.args, self.camera.yaw, input);
                if walk != self.walking {
                    self.walking = walk;
                    lab.walk(walk);
                }
            }
            lab.advance(dt, self.args.fixed_step);
            if let Some(time) = lab.sea_time() {
                self.sea_time = time;
            }
        }
        // The island's walker (#196), as the playground's player walks.
        if let Some(walker) = &mut self.walker {
            walker.walk(wished_walk(&self.args, self.camera.yaw, input));
            walker.advance(f64::from(dt), self.args.fixed_step);
        }
        if let Some(length) = self.args.day {
            self.day_time += self.step;
            // A day, then with the night (D-046) a night as long.
            let cycle = if self.night.is_some() { 2.0 } else { 1.0 };
            self.set_sun_of_day((self.day_time / length.max(1.0)).rem_euclid(cycle));
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
        } else if let Some(speed) = self.args.pan {
            let step = if self.args.fixed_step { 1.0 / 60.0 } else { dt };
            let right = self.camera.forward().cross(Vec3::Y).normalize_or_zero();
            self.camera.position += right * speed * step;
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
        } else if let Some(feet) = self
            .lab
            .as_mut()
            .and_then(lab::Lab::player)
            .map(|p| p.position)
            .or_else(|| self.walker.as_ref().map(|w| w.feet().as_vec3()))
        {
            // The playground (#139) and the island's walker (#196): the right mouse button turns
            // the view round the player, the camera 5 m behind its head along the view; on the
            // island, kept over the ground behind it.
            if input.looking {
                let s = self.camera.sensitivity;
                self.camera.yaw -= input.mouse_delta.0 * s;
                self.camera.pitch = (self.camera.pitch - input.mouse_delta.1 * s).clamp(-1.2, 0.5);
            }
            let head = feet + Vec3::new(0.0, 1.6, 0.0);
            self.camera.position = head - self.camera.forward() * 5.0;
            if let Some(walker) = &self.walker {
                let p = self.camera.position;
                let ground = walker.ground_at(f64::from(p.x), f64::from(p.z)) as f32;
                self.camera.position.y = p.y.max(ground + 0.3);
            }
        } else {
            self.camera.update(input, dt);
        }
        // C: the camera behind the lab's boat, car or aeroplane and over it, looking where it
        // goes (#138, #140, #141); further back from the aeroplane, 7 m long with a 10 m span.
        let (back, over, pitch) = if self.lab.as_mut().is_some_and(lab::Lab::has_plane) {
            (17.0, 3.5, -0.1)
        } else if self.lab.as_mut().is_some_and(lab::Lab::has_birds) {
            // A gull (#184), 1.3 m across, from 3 m behind and a little over it.
            (3.0, 0.7, -0.12)
        } else {
            (8.0, 2.8, -0.18)
        };
        let rocket = self.lab.as_mut().is_some_and(lab::Lab::has_rocket);
        let ship = self.lab.as_mut().is_some_and(lab::Lab::has_ship);
        if self.chase
            && ship
            && let Some(ride) = self.lab.as_mut().and_then(lab::Lab::ride)
        {
            // The spaceship from behind and over it, along its nose and its up as it turns,
            // looking a little ahead of it.
            let forward = ride.rotation * Vec3::NEG_Z;
            let up = ride.rotation * Vec3::Y;
            self.camera.position = ride.position - forward * 24.0 + up * 6.0;
            let to = ride.position + forward * 10.0 - self.camera.position;
            self.camera.yaw = (-to.x).atan2(-to.z);
            self.camera.pitch = to.y.atan2(Vec3::new(to.x, 0.0, to.z).length());
        } else if self.chase
            && rocket
            && let Some(ride) = self.lab.as_mut().and_then(lab::Lab::ride)
        {
            // The rocket (#148) from 30 m off its right, a little behind and over its middle,
            // looking at it: its pitch downrange (−z) crosses the view.
            let middle = ride.position + ride.rotation * Vec3::new(0.0, 6.0, 0.0);
            self.camera.position = middle + Vec3::new(30.0, 2.0, 5.0);
            let to = middle - self.camera.position;
            self.camera.yaw = (-to.x).atan2(-to.z);
            self.camera.pitch = to.y.atan2(Vec3::new(to.x, 0.0, to.z).length());
        } else if self.chase
            && let Some(ride) = self.lab.as_mut().and_then(lab::Lab::ride)
        {
            let forward = ride.rotation * Vec3::NEG_Z;
            let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
            self.camera.position = ride.position - flat * back + Vec3::new(0.0, over, 0.0);
            self.camera.yaw = (-flat.x).atan2(-flat.z);
            self.camera.pitch = pitch;
        }
        self.frame += 1;
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        // In space, on the tank's bench and in the sharpness room (#159), no sky light: a constant fill.
        let bench = self.args.lab == Some(lab::LabScene::TankBench);
        let room = self.args.lab == Some(lab::LabScene::Room);
        let in_space = self.space.is_some() || bench || room;
        // The target's format and the HDR settings (issue #94).
        let hdr = HdrOutput::new(ctx.output.peak, ctx.output.scene_stops, ctx.output.ui_white);
        self.taa.set_output(&ctx.shaders, ctx.output.format, hdr)?;
        // `FORGE_HASH_IMAGES=1` (#161): hashes of the clouds, the scene and TAA's history each
        // frame, logged when their slot comes back, to find where two runs part without the
        // waits that hide a race.
        let hasher = self.hasher.as_ref();
        if let Some(h) = hasher {
            let hashes = h.take(frame.slot);
            if !hashes.is_empty() {
                tracing::info!(frame = ctx.frames_rendered, ?hashes, "image hashes");
            }
            h.begin(&mut frame.graph, frame.slot);
        }
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
            if let Some((at, lights)) = &self.night_now {
                // The night (D-046): which light is the key, what the sky is per unit of, the
                // exposure it is seen at.
                tracing::info!(
                    key = if lights.key_is_sun { "sun" } else { "moon" },
                    sun_elevation = %format_args!("{:.1}", at.sun.y.asin().to_degrees()),
                    moon_elevation = %format_args!("{:.1}", at.moon.y.asin().to_degrees()),
                    moon_lux = %format_args!("{:.3}", at.moon_illuminance),
                    reference_lux = %format_args!("{:.4}", lights.reference),
                    second_weight = %format_args!("{:.3e}", lights.second_weight),
                    ev100 = %format_args!("{:.2}", self.auto_exposure.ev100),
                    "night"
                );
            }
            if self.clouds.is_some() && !self.args.no_cloud_light {
                // The same with the clouds in the sky (#163).
                let c = self.sky.read_irradiance_with_clouds(&ctx.device)?;
                let e = |n: Vec3| luma(sh_irradiance(&c, n));
                tracing::info!(
                    roof = %format_args!("{:.3}", e(Vec3::Y)),
                    floor = %format_args!("{:.3}", e(-Vec3::Y)),
                    wall_to_sun = %format_args!("{:.3}", e(flat)),
                    wall_away = %format_args!("{:.3}", e(-flat)),
                    "sky light with the clouds, per unit of sun illuminance"
                );
            }
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
        // The night's Purkinje shift (D-046) in whichever pass shows the image.
        let night_on = self.night.is_some() && !self.args.no_purkinje;
        self.taa.purkinje = night_on;
        if let Some((_, display)) = self.dlaa.as_mut() {
            display.scotopic = if night_on { 1.0 / exposure } else { 0.0 };
        }
        // Draw jittered into TAA's HDR target, cull with the unjittered camera, resolve
        // through the history and the tone curve into the swapchain.
        let taa_frame = self.taa.begin(
            &mut frame.graph,
            self.camera.projection(ctx.aspect()),
            camera.view_proj,
            camera.position,
            exposure,
        );
        if hasher.is_some() {
            // What the motion vectors take from the CPU (#161), bit for bit.
            let bits = |values: &[f32]| {
                values.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, v| {
                    (h ^ u64::from(v.to_bits())).wrapping_mul(0x0100_0000_01b3)
                })
            };
            tracing::info!(
                frame = ctx.frames_rendered,
                reprojection = bits(&taa_frame.previous_from_current.to_cols_array()),
                view_proj = bits(&camera.view_proj.to_cols_array()),
                exposure = exposure.to_bits(),
                jitter = ?taa_frame.jitter,
                "taa inputs"
            );
        }
        // The camera in the scene frame, where the probes and the rays live (issue #93).
        let camera_in_scene = camera.position.relative_to(self.scene.origin());
        // The movers where they stand at the sea's time (#79): the barrels' ticks up to it (#177);
        // then the island's walker, or its place under the island while nobody walks (#196).
        if self.args.island.is_some() && self.lab.is_none() {
            let mut movers = Vec::new();
            if let Some(barrels) = &mut self.barrels {
                barrels.advance(self.sea_time);
                movers = barrels.transforms();
            }
            movers.extend(
                self.walker
                    .as_ref()
                    .map_or_else(island_walk::Walker::parked, island_walk::Walker::transforms),
            );
            // The sand round the walker after them (#197): its footfalls pressed, its heights
            // sent when they changed, the tiles' ground left to it. It shows the ground, which
            // stands still (a step moves its mover with what it shows), so it has no motion of
            // its own: its pixels move as the camera makes them, and the motion pass skips them.
            let mut still = None;
            if let Some(sand) = &mut self.sand {
                let footfalls = self
                    .walker
                    .as_mut()
                    .map(island_walk::Walker::take_footfalls)
                    .unwrap_or_default();
                sand.follow(
                    self.walker
                        .as_ref()
                        .filter(|_| !self.args.no_sand_window)
                        .map(island_walk::Walker::feet_xz),
                    &footfalls,
                );
                if let Some(field) = sand.field() {
                    self.scene.set_skins(&[Mat4::IDENTITY]);
                    self.scene.set_fields(field);
                }
                self.scene.set_ground_window(sand.rect());
                still = Some(movers.len());
                movers.push(sand.transform());
            }
            self.scene.set_movers_still(&movers, |k| Some(k) == still);
        }
        // The lab's bodies between their last two ticks (#136).
        if let Some(lab) = &self.lab {
            let mut skins = Vec::new();
            self.scene.set_movers(&lab.movers(&mut skins));
            // The skinned creatures' joints (#165).
            self.scene.set_skins(&skins);
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
                self.scene.set_fields(&heights);
            }
            self.beds_seen = changed;
        }
        // The glass tank's liquid (#156): the statistics a frame in this slot asked for, then the
        // substeps the lab's ticks owe it, on the async compute queue.
        // Its buffer imported once, for the simulation and the drawing both.
        let liquid_state = self.liquid.as_ref().map(|l| l.import(&mut frame.graph));
        if let (Some(liquid), Some(state), Some(lab)) =
            (&self.liquid, liquid_state, self.lab.as_mut())
        {
            if let Some(stats) = liquid.take_stats(frame.slot)
                && let Some(tick) = self.liquid_asked[frame.slot.index].take()
            {
                log_liquid(tick, &stats, liquid.tank());
            }
            let mut steps = lab.take_tank_steps();
            if std::mem::take(&mut self.liquid_reset) {
                liquid.reset();
                steps.clear();
            }
            let now = lab.now();
            let every = self.args.liquid_log.max(1);
            let ask = now >= self.liquid_next_log;
            if ask {
                self.liquid_next_log = (now / every + 1) * every;
            }
            self.liquid_asked[frame.slot.index] = ask.then_some(now);
            liquid.simulate(&mut frame.graph, state, frame.slot, &steps, ask);
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
        // (issue #47). The cloud layer's images, this frame's and last (#145).
        let cloud_images = self.clouds.as_ref().map(|c| c.images(&mut frame.graph));
        let sky_view_proj = taa_frame.jittered_projection * self.camera.view_rotation();
        // The night's sky (D-046): the Moon, the second light, the airglow and the stars, a
        // star about a pixel wide.
        let pixel_angle = 2.0 * (0.5 * self.camera.fov_y).tan() / extent.height.max(1) as f32;
        let night = self
            .night
            .as_ref()
            .zip(self.night_now)
            .map(|(night, (at, lights))| night.sky(&at, &lights, pixel_angle, self.night_settings));
        let sky = self.sky.tables(
            &mut frame.graph,
            frame.slot,
            &air,
            SkyParams {
                view_proj: sky_view_proj,
                camera: Vec3::ZERO,
                sun_dir: self.renderer.sun_dir,
                sun_angular_radius: forge_render::starfield::SUN_ANGULAR_RADIUS_1AU,
                luminance_scale: self.renderer.sun_illuminance * exposure,
                aerial_far_km: 8.0,
                night,
            },
            targets.depth,
            taa_frame.color,
            cloud_images.map(|(this, _)| this),
            extent,
        );
        // The clouds marched after the tables (their sun through the air, the sky's light),
        // before the compose lays them over the sky; the weather drifting on a 10 m/s wind.
        // Their shadow on the sun's light for the resolve.
        let mut cloud_shadow = None;
        // The sky's light for the resolve, its reflections and the probes: with the clouds in it
        // when there are clouds (#163). The clouds themselves and the water keep the clear sky's
        // (the water's reflections take the clouds from the screen), and so do the rays the
        // water traces.
        let mut scene_light = sky.light;
        if let (Some(clouds), Some(images)) = (&self.clouds, cloud_images) {
            let time = self.sea_time as f32;
            let params = CloudParams {
                camera: [self.camera.position.x, self.camera.position.z],
                drift: [8.0 * time, 6.0 * time],
                previous: self.clouds_previous,
                ..CloudParams::fair(self.args.clouds)
            };
            clouds.march(&mut frame.graph, frame.slot, &sky, params, images);
            cloud_shadow = Some(clouds.shadow(&mut frame.graph, frame.slot, &sky, params));
            if !self.args.no_cloud_light {
                let table = clouds.sky_table(&mut frame.graph, frame.slot, &sky);
                scene_light = self.sky.light_with(&mut frame.graph, &sky, table);
            }
            self.clouds_previous = sky_view_proj;
        }
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
        let probe_extent = self.probes.as_ref().map(|p| p.irradiance_extent());
        let probes = match &mut self.probes {
            Some(probes) if self.probes_on && self.sky_light && !in_space => {
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
                    scene_light,
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
        // The sky's light (in space, its constant fill), occluded by what the depth shows around
        // each pixel (issue #48).
        let occlusion = ((self.sky_light || in_space) && self.ao_on).then(|| {
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
        if let Some(h) = hasher {
            use forge_render::debug_hash::HashKind;
            if let (Some(light), Some([w, hgt])) = (probes.as_ref(), probe_extent) {
                let atlas = forge_gpu::vk::Extent2D {
                    width: w,
                    height: hgt,
                };
                h.add(
                    &mut frame.graph,
                    light.irradiance(),
                    HashKind::Float4,
                    atlas,
                );
            }
            if let Some(ao) = occlusion {
                h.add(&mut frame.graph, ao, HashKind::Float4, extent);
            }
        }
        // The sun's soft shadow denoised by NVIDIA's SIGMA (#172) or AMD's FidelityFX (#173),
        // D-049: a ray a pixel before the resolve, which then reads the result in place of its
        // own ray. The motion vectors come first, for the denoiser's reprojection (they are
        // reused below).
        let mut early_motion = None;
        let denoising = self.sun_shadow_on.filter(|_| {
            self.flags.has(CullFlags::SHADOWS)
                && self.renderer.sun_angular_radius > 0.0
                && self.scene.rays().is_some()
        });
        let mut sun_shadow = None;
        if let Some(index) = denoising {
            let denoiser = &mut self.sun_shadows[index];
            if taa_frame.reset {
                denoiser.reset_history();
            }
            let shadow_camera = forge_render::SunShadowCamera {
                view: self.camera.view_rotation(),
                projection: self.camera.projection(ctx.aspect()),
                position: camera.position,
                jitter: taa_frame.jitter,
            };
            let shadow_frame = denoiser.frame(
                shadow_camera,
                self.renderer.sun_dir,
                self.renderer.noise_frame,
                self.step * 1000.0,
            );
            let closest = denoiser.needs_closest_hit();
            if let Some(rays) = self.renderer.trace_sun_shadow(
                &mut frame.graph,
                frame.slot,
                targets,
                extent,
                forge_render::SunShadowDenoiser::view_z_row(&shadow_camera),
                self.renderer.sun_angular_radius.tan(),
                closest,
            ) {
                let motion = self
                    .taa
                    .motion_vectors(&mut frame.graph, &taa_frame, targets.depth);
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
                early_motion = Some(motion);
                let denoiser: &'f forge_render::SunShadowDenoiser = &self.sun_shadows[index];
                sun_shadow = Some(denoiser.denoise(
                    &mut frame.graph,
                    frame.slot,
                    rays,
                    motion,
                    &shadow_frame,
                )?);
                // The shadow rays' three images and the denoised shadow (#204: a frame that lays
                // the transients out again lost them in the serial frame).
                if let (Some(h), Some(out)) = (hasher, sun_shadow) {
                    use forge_render::debug_hash::HashKind;
                    h.add(&mut frame.graph, rays.penumbra, HashKind::Float4, extent);
                    h.add(&mut frame.graph, rays.view_z, HashKind::Float4, extent);
                    h.add(
                        &mut frame.graph,
                        rays.normal_roughness,
                        HashKind::Float4,
                        extent,
                    );
                    h.add(&mut frame.graph, out, HashKind::Float4, extent);
                }
            }
        }
        // The mirror rays' history (#176, D-050): the glass's reflections settled before TAA.
        let steadied = !self.args.no_reflection_history
            && self.sky_light
            && !in_space
            && self.flags.has(CullFlags::SKY_REFLECTIONS)
            && self.flags.has(CullFlags::RAY_REFLECTIONS)
            && self.scene.rays().is_some();
        let mut reflection_history = None;
        if steadied {
            if taa_frame.reset {
                self.reflection_history.reset_history();
            }
            self.reflection_history.advance();
            let history: &'f forge_render::ReflectionHistory = &self.reflection_history;
            reflection_history = Some(history.frame(
                &mut frame.graph,
                frame.slot,
                taa_frame.previous_from_current,
                taa_frame.jitter,
                // Reversed-Z, infinite: clip z is the near plane.
                self.camera.projection(ctx.aspect()).w_axis.z,
                self.scene.first_mover(),
            ));
        } else {
            self.reflection_history.reset_history();
        }
        self.renderer.resolve(
            &mut frame.graph,
            frame.slot,
            targets,
            taa_frame.color,
            extent,
            None,
            AmbientLight {
                sky: (self.sky_light && !in_space).then_some(scene_light),
                occlusion,
                probes,
                wet_ground,
                movers: targets.movers,
                clouds: cloud_shadow,
                sun_shadow,
                reflection_history,
            },
        );
        if let Some(space) = self.space.as_mut() {
            // Space's sky where nothing was drawn: the stars, the sun's disc, the planet and its
            // air seen from orbit; no haze over the ship.
            let planet = space.planet.frame(
                &mut frame.graph,
                frame.slot,
                space.view,
                self.renderer.sun_dir,
            );
            space.starfield.draw(
                &mut frame.graph,
                taa_frame.color,
                targets.depth,
                extent,
                sky_view_proj,
                self.renderer.sun_dir,
                exposure,
                Some(planet),
            );
        } else if !bench {
            self.sky.compose(
                &mut frame.graph,
                &sky,
                targets.depth,
                taa_frame.color,
                extent,
            );
        }
        if let Some(h) = hasher {
            use forge_render::debug_hash::HashKind;
            if let Some((clouds, _)) = cloud_images {
                h.add(&mut frame.graph, clouds, HashKind::Float4, extent);
            }
            h.add(&mut frame.graph, taa_frame.color, HashKind::Float4, extent);
            h.add(&mut frame.graph, targets.depth, HashKind::Depth, extent);
        }
        // The splashes' reactive mask for TAA (#107), when spray is alive.
        let mut reactive = None;
        if let (Some((cascades, surface, _)), Some(waves)) = (&self.water, &waves) {
            // The barrels nearest the camera part the rivers' flow, and make waves in still
            // water (#107).
            let camera = Vec2::new(camera_in_scene.x, camera_in_scene.z);
            if let Some(barrels) = self.barrels.as_ref().filter(|_| !self.args.no_floaters) {
                surface.set_floaters(&barrels.floaters(camera));
            }
            // The flood's water as the lab's world holds it (#144): through the GPU's finer layer
            // that shadows its columns (#162), or the columns themselves.
            let columns = if self.args.no_gpu_water {
                None
            } else {
                self.lab.as_mut().and_then(lab::Lab::columns)
            };
            if let Some((now, pool)) = columns {
                if self.shallow.is_none() {
                    let grid = forge_render::ShallowGrid {
                        columns: [pool.size[0] as u32, pool.size[1] as u32],
                        spacing: pool.spacing,
                        origin: [pool.origin[0] as f32, pool.origin[1] as f32],
                        ratio: self.args.gpu_water_ratio.max(1),
                    };
                    let mut layer =
                        forge_render::ShallowLayer::new(&ctx.device, &ctx.shaders, grid)?;
                    layer.params = forge_render::ShallowParams {
                        substeps: self.args.gpu_water_substeps.max(1),
                        rate: self.args.gpu_water_rate.clamp(0.0, 1.0),
                        friction: pool.friction,
                        gravity: pool.gravity,
                    };
                    tracing::info!(
                        cells = ?grid.cells(),
                        cell_m = grid.fine_spacing(),
                        mib = format!("{:.1}", layer.bytes() as f64 / 1_048_576.0),
                        "the GPU's water layer"
                    );
                    self.shallow = Some(layer);
                }
                let layer = self.shallow.as_ref().expect("made above");
                // The ticks the columns took since the last frame; anything else (a reset, a
                // replay, a correction from the server) starts the layer again from them.
                let ticks = match self.shallow_tick {
                    Some(last) if now >= last && now - last <= 8 => (now - last) as u32,
                    _ => {
                        layer.reset();
                        0
                    }
                };
                self.shallow_tick = Some(now);
                if let Some(stats) = layer.take_stats(frame.slot) {
                    tracing::info!(
                        tick = now,
                        volume_m3 = format!("{:.3}", stats.volume),
                        columns_m3 = format!("{:.3}", stats.columns),
                        gap = format!(
                            "{:.3}%",
                            100.0 * (stats.volume - stats.columns) / stats.columns.max(1e-9)
                        ),
                        mean_gap_mm = format!("{:.2}", 1000.0 * stats.mean_gap),
                        largest_gap_mm = format!("{:.1}", 1000.0 * stats.largest_gap),
                        "the GPU's water layer"
                    );
                }
                let (u, w) = pool.faces();
                let state = layer.import(&mut frame.graph);
                layer.simulate(
                    &mut frame.graph,
                    state,
                    frame.slot,
                    forge_render::ShallowColumns {
                        depth: &pool.depth,
                        u,
                        w,
                        floats: &pool.displaced,
                        bed: &pool.bed,
                    },
                    ticks,
                    forge_sim::TICK,
                    ctx.frames_rendered.is_multiple_of(60),
                );
                let grid = layer.grid();
                surface.set_pool_on_gpu(forge_render::WaterPoolOnGpu {
                    buffer: state.buffer,
                    samples: state.samples,
                    origin: grid.fine_origin(),
                    spacing: grid.fine_spacing(),
                    size: grid.cells(),
                });
            } else if let Some(pool) = self.lab.as_mut().and_then(lab::Lab::pool) {
                surface.set_pool(&WaterPool {
                    origin: pool.origin,
                    spacing: pool.spacing,
                    size: pool.size,
                    samples: &pool.samples,
                });
            }
            // The foam the splashes left where their drops landed (#107's polish), faded, for the
            // water to whiten by and the wakes to ring from; this frame's landings add to it
            // after.
            let splash_foam = self
                .splashes
                .as_ref()
                .map(|s| s.foam(&mut frame.graph, camera_in_scene, self.sea_time_submitted));
            let wakes = self
                .wakes
                .as_ref()
                .zip(self.barrels.as_ref())
                .map(|(wakes, barrels)| {
                    wakes.update(
                        &mut frame.graph,
                        frame.slot,
                        &barrels.wakes(camera),
                        splash_foam.filter(|_| !self.args.no_drop_rings),
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
                    clouds: cloud_shadow,
                    cloud_image: cloud_shadow.and(cloud_images).map(|(this, _)| this),
                    splash_foam,
                    sea_water: self.args.sea_water,
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
                        // The water's own sky (#163): a hit replaces the sky the water drew
                        // there (the clear table under the screen's clouds), which this
                        // subtracts; the clouded table's brighter sky took more than was there.
                        sky: (self.sky_light && !in_space).then_some(sky.light),
                        occlusion: None,
                        probes,
                        wet_ground: None,
                        movers: targets.movers,
                        clouds: None,
                        sun_shadow: None,
                        reflection_history: None,
                    },
                );
            }
        }
        // The spray where it splashes (#107), over the water and its reflections; and the ground a
        // slipping wheel throws, with or without water (#192: the yard's).
        if let Some(splashes) = &self.splashes {
            let mut sources = self.falls.clone();
            if let Some(barrels) = &self.barrels {
                barrels.splashes(&mut sources);
            }
            // The flood's front and its water striking the walls (#162), the yard's slipping
            // wheels' spray (#192).
            if let Some(lab) = self.lab.as_mut() {
                lab.splashes(&mut sources);
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
                    // The sea's clock, as its waves were given it; without a sea, the same.
                    time: if self.water.is_some() {
                        self.sea_time_submitted
                    } else {
                        self.sea_time as f32
                    },
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
        // The motion vectors, before the tank's water, whose scene behind them a TAA takes. They
        // need only the depth and the cameras. Before the resolve when SIGMA denoised the sun's
        // shadow, which reprojects through them (#172).
        let motion = match early_motion {
            Some(motion) => motion,
            None => self
                .taa
                .motion_vectors(&mut frame.graph, &taa_frame, targets.depth),
        };
        if let Some(h) = hasher {
            h.add(
                &mut frame.graph,
                motion,
                forge_render::debug_hash::HashKind::Float4,
                extent,
            );
        }
        // The movers' own motion over the camera's (#79).
        if early_motion.is_none() && !self.args.no_mover_motion {
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
        // The glass tank and its water (#156), over the scene and the sky; its reactive mask for TAA
        // (the spray's, where there is spray, comes first: the tank has none).
        if let (Some(liquid), Some(state)) = (&self.liquid, liquid_state) {
            let background = bench.then(|| {
                BENCH_BACKGROUND * (self.renderer.sun_illuminance * exposure / std::f32::consts::PI)
            });
            let mut scene = liquid.copy_scene(
                &mut frame.graph,
                background,
                taa_frame.color,
                targets.depth,
                extent,
            );
            if let Some(behind) = &mut self.behind_taa {
                let behind_frame = behind.follow(&taa_frame, scene);
                scene = self.behind_taa.as_ref().expect("just used").resolve_hdr(
                    &mut frame.graph,
                    "liquid/scene TAA",
                    &behind_frame,
                    targets.depth,
                    motion,
                );
            }
            let mask = liquid.draw(
                &mut frame.graph,
                state,
                frame.slot,
                &sky,
                LiquidDrawParams {
                    view_proj: taa_frame.jittered_projection * self.camera.view_rotation(),
                    corner: lab::tank::corner(bench) - camera_in_scene,
                    sun_dir: self.renderer.sun_dir,
                    sun_radiance: self.renderer.sun_color
                        * (self.renderer.sun_illuminance * exposure),
                    sky_scale: self.renderer.sun_illuminance * exposure,
                    // The bench's background: a dull violet card in the sun.
                    background,
                    camera: camera_in_scene,
                    frame: self.renderer.frame_address(frame.slot),
                    movers: targets.movers,
                    mode: self.liquid_mode,
                },
                scene,
                taa_frame.color,
                targets.depth,
                extent,
            );
            reactive = reactive.or(Some(mask));
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
        let dlaa_on = self.dlaa_on;
        if let Some((dlss, display)) = self.dlaa.as_mut().filter(|_| dlaa_on) {
            // DLAA (D-045): the jittered frame, its depth and motion to DLSS, its HDR image through
            // the tone curve with bloom.
            let bloom = self
                .bloom_on
                .then(|| self.bloom.draw(&mut frame.graph, taa_frame.color, extent));
            let upscaled = dlss.upscale(
                &mut frame.graph,
                ctx.frames_rendered,
                &taa_frame,
                forge_render::UpscaleCamera {
                    projection: self.camera.projection(ctx.aspect()),
                    near: self.camera.near,
                    vertical_fov: self.camera.fov_y,
                },
                targets.depth,
                motion,
                exposure,
            )?;
            display.draw(
                &mut frame.graph,
                upscaled,
                frame.target,
                ctx.extent(),
                self.tonemap,
                bloom.map(|b| (b, self.taa.bloom_strength)),
            );
            return Ok(());
        }
        let bloom = self
            .bloom_on
            .then(|| self.bloom.draw(&mut frame.graph, taa_frame.color, extent));
        let history = self.taa.resolve(
            &mut frame.graph,
            &taa_frame,
            targets.depth,
            motion,
            frame.target,
            self.tonemap,
            bloom,
            reactive.filter(|_| !self.args.no_reactive),
        );
        if let Some(h) = hasher {
            use forge_render::debug_hash::HashKind;
            h.add(&mut frame.graph, motion, HashKind::Float4, extent);
            h.add(&mut frame.graph, history, HashKind::Float4, extent);
            h.finish(&mut frame.graph);
        }
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
        let title = match &mut self.lab {
            Some(lab) => format!("{title} | {}", lab.title()),
            None => title,
        };
        let title = match &mut self.barrels {
            Some(barrels) => format!("{title} | {}", barrels.title()),
            None => title,
        };
        let title = match &mut self.walker {
            Some(walker) => format!("{title} | {}", walker.title()),
            None => title,
        };
        let title = match self.sand.as_mut().filter(|_| self.walker.is_some()) {
            Some(sand) => format!("{title} | {}", sand.title()),
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

/// The walk the keys ask of a player (#139) or the island's walker (#196): WASD along the view
/// turned `yaw`, Shift to run; or `--walk`. M/s along the ground, world x and z.
fn wished_walk(args: &Args, yaw: f32, input: &Input) -> [f32; 2] {
    if let Some(w) = &args.walk {
        return [w[0], w.get(1).copied().unwrap_or(0.0)];
    }
    let (sin, cos) = yaw.sin_cos();
    let (forward, right) = (Vec2::new(-sin, -cos), Vec2::new(cos, -sin));
    let keys = |k: KeyCode| f32::from(u8::from(input.is_down(k)));
    let wish = forward * (keys(KeyCode::KeyW) - keys(KeyCode::KeyS))
        + right * (keys(KeyCode::KeyD) - keys(KeyCode::KeyA));
    let speed = if input.is_down(KeyCode::ShiftLeft) {
        lab::RUN_SPEED
    } else {
        lab::WALK_SPEED
    };
    (wish.normalize_or_zero() * speed).to_array()
}

/// The glass tank's line in the log (#156): what is left of the water, where its surface stands
/// against where its volume puts it over the floor, how far its front has run, how fast it moves,
/// how high the water behind the gate stands (its particles' volume over the floor there: with the
/// hole, its fall gives the outflow), how much outflow the pressure solve left (its residual, as a
/// share of what it had to undo, and the worst cell's, m/s), and the digest a replay must match.
fn log_liquid(tick: u64, stats: &LiquidStats, tank: &LiquidTank) {
    let start = tank.water.as_vec3() * tank.cell;
    let volume = start.x * start.y * start.z;
    let level = settled_level(tank, volume);
    let particle = tank.cell.powi(3) / 8.0;
    let behind = tank.gate.map_or(0.0, |g| {
        stats.behind as f32 * particle / (g[0] * tank.size.z)
    });
    tracing::info!(
        tick,
        particles = stats.particles,
        lost = stats.lost,
        level_mm = format!("{:.1}", stats.level * 1e3),
        settled_mm = format!("{:.1}", level * 1e3),
        volume = format!("{:.4}", stats.volume / volume),
        front_mm = format!("{:.0}", stats.front * 1e3),
        height_mm = format!("{:.1}", stats.mean_height * 1e3),
        rms_speed = format!("{:.3}", stats.mean_speed2.sqrt()),
        max_speed = format!("{:.2}", stats.max_speed),
        behind_mm = format!("{:.1}", behind * 1e3),
        residual = format!("{:.4}", stats.residual),
        residual_max = format!("{:.4}", stats.residual_max),
        digest = format!("{:08x}{:08x}", stats.digest[0], stats.digest[1]),
        "liquid"
    );
}

/// Where `volume` of water stands over the tank's floor, less what the blocks in it take as it
/// rises past them.
fn settled_level(tank: &LiquidTank, volume: f32) -> f32 {
    let held = |h: f32| {
        tank.size.x * tank.size.z * h
            - tank
                .obstacles
                .iter()
                .map(|o| {
                    (o.max.x - o.min.x) * (o.max.z - o.min.z) * (h.min(o.max.y) - o.min.y).max(0.0)
                })
                .sum::<f32>()
    };
    if tank.obstacles.is_empty() {
        return volume / (tank.size.x * tank.size.z);
    }
    let (mut low, mut high) = (0.0, tank.size.y);
    for _ in 0..40 {
        let mid = 0.5 * (low + high);
        if held(mid) < volume {
            low = mid;
        } else {
            high = mid;
        }
    }
    0.5 * (low + high)
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
    // `--textures` (the owner's question of 2026-10-03: are the textures what looks fuzzy?).
    let mode = TEXTURES.get().copied().unwrap_or_default();
    RenderLayer {
        color_a: a,
        color_b: b,
        albedo_texture: (mode != TextureMode::None).then_some(albedo),
        normal_texture: (mode == TextureMode::Full).then_some(normal),
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
        // The physics lab's barrels and balls (#136): red paint and orange rubber.
        let red_paint = add(
            "painted metal (red)",
            RenderLayer {
                color_a: [0.32, 0.045, 0.03],
                color_b: [0.28, 0.05, 0.035],
                roughness: RenderLayer::roughness_for_power(60.0),
                specular: 0.25,
                ..RenderLayer::default()
            },
        );
        let rubber = add(
            "rubber (orange)",
            RenderLayer {
                color_a: [0.62, 0.2, 0.03],
                color_b: [0.55, 0.22, 0.04],
                roughness: RenderLayer::roughness_for_power(20.0),
                specular: 0.04,
                ..RenderLayer::default()
            },
        );
        // The sea scene's (#138): crates of pale wood, logs in their bark, a jetty of grey
        // weathered planks on concrete pillars.
        let crate_wood = add(
            "wood (crate)",
            textured(
                concrete,
                [0.86, 0.62, 0.38],
                [0.78, 0.56, 0.34],
                1.5,
                8.0,
                0.03,
            ),
        );
        let bark = add(
            "bark",
            RenderLayer {
                cavity: 0.15,
                // The rock texture (the name `rock` is its row by now).
                ..textured(
                    sets[0],
                    [0.62, 0.45, 0.32],
                    [0.55, 0.4, 0.3],
                    1.0,
                    6.0,
                    0.02,
                )
            },
        );
        let deck_wood = add(
            "wood (weathered)",
            textured(
                concrete,
                [0.78, 0.72, 0.64],
                [0.72, 0.67, 0.6],
                2.0,
                8.0,
                0.03,
            ),
        );
        // The break scene's (#142): bricks of fired clay, each its own shade between two reds;
        // a gantry painted yellow; a wrecking ball and its chain of dark, worn steel.
        let clay = add(
            "brick (clay)",
            textured(
                concrete,
                [0.46, 0.16, 0.09],
                [0.66, 0.3, 0.17],
                0.6,
                6.0,
                0.03,
            ),
        );
        let yellow_paint = add(
            "painted metal (yellow)",
            RenderLayer {
                color_a: [0.7, 0.45, 0.03],
                color_b: [0.7, 0.45, 0.03],
                roughness: RenderLayer::roughness_for_power(60.0),
                specular: 0.25,
                ..RenderLayer::default()
            },
        );
        let steel = add(
            "steel (dull)",
            RenderLayer {
                color_a: [0.05, 0.05, 0.055],
                color_b: [0.06, 0.058, 0.055],
                roughness: RenderLayer::roughness_for_power(25.0),
                specular: 0.35,
                reflectance: 0.15,
                ..RenderLayer::default()
            },
        );
        // Its column: concrete, and paler and finer where it broke (the pieces' second
        // section).
        let column = add(
            "concrete (column)",
            textured(
                concrete,
                [0.72, 0.72, 0.7],
                [0.64, 0.64, 0.62],
                2.0,
                12.0,
                0.05,
            ),
        );
        add(
            "concrete (column): broken",
            textured(
                concrete,
                [0.88, 0.86, 0.8],
                [0.8, 0.78, 0.73],
                0.7,
                6.0,
                0.02,
            ),
        );
        // The flight's runway (#141): dark asphalt.
        let asphalt = add(
            "asphalt",
            textured(
                concrete,
                [0.24, 0.24, 0.26],
                [0.22, 0.22, 0.24],
                3.0,
                8.0,
                0.03,
            ),
        );
        // The playground's player (#139): a blue body, a dark glossy visor.
        let player_paint = add(
            "painted (blue)",
            RenderLayer {
                color_a: [0.05, 0.16, 0.42],
                color_b: [0.05, 0.16, 0.42],
                roughness: RenderLayer::roughness_for_power(40.0),
                specular: 0.2,
                ..RenderLayer::default()
            },
        );
        // The slimes (#179, #180), the tropical island's in its four flavours: jelly seen
        // through, a little cloudy, each followed by its eyes' row (section 1, black and glossy)
        // and its nucleus's (section 2, the jelly's tint darkened as the island's: 0.42 of it).
        let mut slime_rows = Vec::new();
        for (name, tint) in lab::SLIME_FLAVOURS {
            slime_rows.push(add(
                name,
                RenderLayer {
                    class: ShadingClass::Jelly,
                    // What 0.3 m of it lets through (the island mixed what lies behind with
                    // three quarters of the tint), and its cloud's colour, the tint.
                    color_a: tint.map(|t| 0.25 + 0.75 * t),
                    color_b: tint,
                    // A cloud scattering 0.8 per metre (`bubbles` of a millimetre, as the ice's).
                    bubbles: 0.00053,
                    roughness: RenderLayer::roughness_for_power(400.0),
                    specular: 0.5,
                    reflectance: 0.025,
                    ..RenderLayer::default()
                },
            ));
            add(
                "slime: eyes",
                RenderLayer {
                    color_a: [0.02, 0.02, 0.04],
                    color_b: [0.02, 0.02, 0.04],
                    roughness: RenderLayer::roughness_for_power(300.0),
                    specular: 0.6,
                    ..RenderLayer::default()
                },
            );
            add(
                "slime: nucleus",
                RenderLayer {
                    color_a: tint.map(|t| 0.42 * t),
                    color_b: tint.map(|t| 0.42 * t),
                    ..RenderLayer::default()
                },
            );
        }
        let visor = add(
            "visor",
            RenderLayer {
                color_a: [0.02, 0.02, 0.025],
                color_b: [0.02, 0.02, 0.025],
                roughness: RenderLayer::roughness_for_power(300.0),
                specular: 0.6,
                ..RenderLayer::default()
            },
        );
        // The rocket's (#148), after the others so their numbers stay.
        let white_paint = add(
            "painted (white)",
            RenderLayer {
                color_a: [0.62, 0.62, 0.6],
                color_b: [0.6, 0.6, 0.58],
                roughness: RenderLayer::roughness_for_power(60.0),
                specular: 0.25,
                ..RenderLayer::default()
            },
        );
        // The tank's bench (#156): a floor of 10 cm squares to measure by, a tint a quadrant.
        let squares = textures::checker(512);
        let squares = (textures.add(&squares[0])?, textures.add(&squares[1])?);
        let mut bench =
            |name: &str, tint: [f32; 3]| add(name, textured(squares, tint, tint, 1.0, 20.0, 0.1));
        let bench_blue = bench("bench (blue)", [0.3, 0.45, 0.95]);
        let bench_violet = bench("bench (violet)", [0.62, 0.38, 0.9]);
        let bench_sand = bench("bench (sand)", [0.9, 0.7, 0.38]);
        let bench_green = bench("bench (green)", [0.35, 0.78, 0.42]);
        // The sharpness room (#159): matte white and black in flat colour, and a floor of black and
        // white squares.
        let matte = |c: f32| RenderLayer {
            color_a: [c; 3],
            color_b: [c; 3],
            roughness: RenderLayer::roughness_for_power(4.0),
            specular: 0.02,
            ..RenderLayer::default()
        };
        let room_white = add("matte (white)", matte(0.8));
        let room_black = add("matte (black)", matte(0.03));
        let squares = textures::black_and_white(512);
        let squares = (textures.add(&squares[0])?, textures.add(&squares[1])?);
        let room_floor = add(
            "squares (black and white)",
            textured(squares, [1.0; 3], [1.0; 3], 1.0, 4.0, 0.02),
        );
        // The dogs' paw prints (#167's foot-down events): damp earth, darker than any ground
        // they walk.
        let print = add(
            "paw print (damp earth)",
            RenderLayer {
                color_a: [0.10, 0.075, 0.055],
                color_b: [0.08, 0.06, 0.045],
                roughness: RenderLayer::roughness_for_power(8.0),
                specular: 0.03,
                ..RenderLayer::default()
            },
        );
        // The yard's beds (#185), the prints drawn by their own relief: fresh snow, white with
        // a faint grain and sheen; damp sand, the island's darkened, its grain faint.
        let snow = add(
            "snow (fresh)",
            RenderLayer {
                albedo_texture: None,
                normal_strength: 0.25,
                ..textured(
                    concrete,
                    [0.86, 0.88, 0.92],
                    [0.84, 0.86, 0.91],
                    0.5,
                    6.0,
                    0.06,
                )
            },
        );
        let damp_sand = add(
            "sand (damp)",
            RenderLayer {
                normal_strength: 0.3,
                ..textured(
                    concrete,
                    [0.70, 0.60, 0.43],
                    [0.76, 0.66, 0.48],
                    2.0,
                    10.0,
                    0.04,
                )
            },
        );
        // Mud (#186): dark wet soil with a sheen, its grain faint.
        let mud = add(
            "mud (wet)",
            RenderLayer {
                normal_strength: 0.2,
                ..textured(
                    concrete,
                    [0.34, 0.25, 0.17],
                    [0.38, 0.28, 0.19],
                    1.5,
                    60.0,
                    0.3,
                )
            },
        );
        let mut by_prop = HashMap::from([
            ("lab-floor", concrete_grey),
            ("lab-block", sandstone),
            ("lab-barrel", red_paint),
            ("lab-rock-1", rock),
            ("lab-rock-2", rock),
            ("lab-rock-3", rock),
            ("lab-ball", rubber),
            ("lab-crate", crate_wood),
            ("lab-log", bark),
            ("lab-pillar", concrete_grey),
            ("lab-deck", deck_wood),
            ("lab-slab", concrete_grey),
            ("lab-ramp", deck_wood),
            ("lab-platform", red_paint),
            ("lab-player", player_paint),
            ("lab-visor", visor),
            ("lab-runway", asphalt),
            ("lab-field", grass),
            ("lab-brick", clay),
            ("lab-post", yellow_paint),
            ("lab-beam", yellow_paint),
            ("lab-wrecking-ball", steel),
            ("lab-chain", steel),
            ("lab-column", column),
            ("lab-column-piece", column),
            ("lab-pole", metal),
            ("lab-dam-wall-x", concrete_grey),
            ("lab-dam-wall-z", concrete_grey),
            ("lab-gate", red_paint),
            ("lab-dam-block", concrete_grey),
            ("lab-hut", brick_red),
            ("lab-domino", crate_wood),
            ("lab-bank", sandstone),
            ("lab-bridge-panel", crate_wood),
            ("lab-rocket", white_paint),
            ("lab-rocket-fins", red_paint),
            ("lab-pad", concrete_grey),
            ("lab-rope", bark),
            ("lab-line", white_paint),
            ("lab-tank-table", deck_wood),
            ("lab-tank-bar-x", steel),
            ("lab-tank-bar-y", steel),
            ("lab-tank-bar-z", steel),
            ("lab-tank-gate", red_paint),
            ("lab-tank-wall", red_paint),
            ("lab-tank-shutter", steel),
            ("lab-tank-cube", concrete_grey),
            ("lab-tank-post", concrete_grey),
            ("lab-bench-blue", bench_blue),
            ("lab-bench-violet", bench_violet),
            ("lab-bench-sand", bench_sand),
            ("lab-bench-green", bench_green),
            ("lab-room-floor", room_floor),
            ("lab-room-wall", room_white),
            ("lab-room-side", room_white),
            ("lab-room-target", room_black),
            ("lab-room-board", room_white),
            ("lab-room-board-target", room_black),
            ("lab-step", sandstone),
            ("lab-dog-ramp", deck_wood),
            ("lab-landing", sandstone),
            ("lab-print", print),
            ("lab-snow", snow),
            ("lab-sand", damp_sand),
            ("lab-mud", mud),
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
            ("island-log", bark),
            ("island-crate", crate_wood),
        ]);
        for ((name, _), row) in lab::SLIME_FLAVOURS.into_iter().zip(slime_rows) {
            by_prop.insert(name, row);
        }
        // The materials' patches (#203): each row as physics reads it, drawn as the lab draws
        // its brick, its planks, its sand (dry: paler), its snow and ice. One record for both
        // (D-007).
        for row in &lab::MATERIAL_ROWS {
            let look = match &row.name["lab-mat-".len()..] {
                "brick" => table.get(clay).render,
                "wood" => table.get(deck_wood).render,
                "sand" => RenderLayer {
                    color_a: [0.80, 0.70, 0.52],
                    color_b: [0.86, 0.76, 0.57],
                    ..table.get(damp_sand).render
                },
                "snow" => table.get(snow).render,
                _ => forge_render::material::stock::ice().render,
            };
            let id = table.add(Material {
                physics: row.physics,
                tags: row.tags,
                ..Material::new(row.name, look)
            });
            by_prop.insert(row.name, id);
        }
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
        let mut valley_sets: [Option<[TextureData; 2]>; 8] = Default::default();
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
                        6 => textures::karst(21, 512),
                        _ => textures::grus(22, 512),
                    });
                });
            }
        });
        let mut ids = Vec::new();
        for set in valley_sets.iter().flatten() {
            ids.push((self.textures.add(&set[0])?, self.textures.add(&set[1])?));
        }
        let [
            gravel,
            scree,
            scrub,
            shingle,
            granite,
            limestone,
            karst,
            grus,
        ] = [
            ids[0], ids[1], ids[2], ids[3], ids[4], ids[5], ids[6], ids[7],
        ];
        tracing::info!(
            ms = start.elapsed().as_millis(),
            "island textures: gravel, scree, scrub, shingle, granite, limestone, karst and grus"
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
            (
                // Grus on the granite's gentle ground near its outcrops (#135): coarse buff
                // sand of feldspar, quartz and mica, small pieces of granite lying on it.
                "island: grus",
                textured(
                    grus,
                    [0.56, 0.53, 0.49],
                    [0.62, 0.58, 0.53],
                    3.0,
                    12.0,
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
    /// Adds `rows` one after the other, the first for `prop`: a mesh with sections draws
    /// section `s` with the row `s` after its own (a model's materials, #138).
    fn add_rows(&mut self, prop: &'static str, rows: Vec<(String, RenderLayer)>) {
        let mut first = None;
        for (name, layer) in rows {
            let id = self.table.add(Material::new(&name, layer));
            first.get_or_insert(id);
        }
        if let Some(first) = first {
            self.by_prop.insert(prop, first);
        }
    }

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
    let props = if let Some(scene) = args.lab {
        lab::props(scene)
    } else if args.island.is_some() {
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
    let pages_in_memory = args.gallery || args.lab.is_some() || args.stream_pool == 0;
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
    /// Grus: the coarse sand the granite rots into, on its gentle ground near the bare
    /// rock (#135).
    pub const GRUS: u8 = 16;
    /// How many layers there are.
    pub const COUNT: u8 = 17;
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

/// `--textures`: what the props' procedural textures keep, to compare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
enum TextureMode {
    /// The colour and the relief (normal) maps.
    #[default]
    Full,
    /// The colour map alone: flat surfaces.
    Flat,
    /// Neither: each material's plain colours.
    None,
}

/// Set once from `--textures` before the materials are made.
static TEXTURES: std::sync::OnceLock<TextureMode> = std::sync::OnceLock::new();

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
/// around. Made once a process for the island's heights (the loading thread makes them, #201).
fn island_stones(
    height: &Field2<f32>,
    ribbons: &[forge_procgen::Ribbon],
    channels: &forge_procgen::Channels,
) -> Vec<forge_procgen::Stone> {
    static MADE: std::sync::Mutex<Option<(u64, Vec<forge_procgen::Stone>)>> =
        std::sync::Mutex::new(None);
    let key = height.digest() ^ u64::from(height.size);
    let mut made = MADE.lock().expect("the island's river stones");
    if let Some((made_for, stones)) = made.as_ref()
        && *made_for == key
    {
        return stones.clone();
    }
    let stones = forge_procgen::stones(ribbons, channels, height, RIVER_STONES);
    *made = Some((key, stones.clone()));
    stones
}

/// The stones beside the steeper rivers' water, on their gravel (#118), made once a process for
/// the island's heights (#201).
fn island_bank_stones(
    height: &Field2<f32>,
    ribbons: &[forge_procgen::Ribbon],
    channels: &forge_procgen::Channels,
) -> Vec<forge_procgen::Stone> {
    static MADE: std::sync::Mutex<Option<(u64, Vec<forge_procgen::Stone>)>> =
        std::sync::Mutex::new(None);
    let key = height.digest() ^ u64::from(height.size);
    let mut made = MADE.lock().expect("the island's bank stones");
    if let Some((made_for, stones)) = made.as_ref()
        && *made_for == key
    {
        return stones.clone();
    }
    let stones = forge_procgen::bank_stones(ribbons, channels, height, RIVER_STONES ^ 0xba);
    *made = Some((key, stones.clone()));
    stones
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
        back: 0.0,
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
    // How far back from the mouth at point `k` (against `direction`) the river's water reaches:
    // along its channel, its points walked away from the mouth (up the river, or down it from a
    // lake's outlet), while each lies further back than the last and its middle within its half
    // width of the line, at most 200 m. A straight band of a brook's water 200 m long had crossed
    // a lake where its channel turned away (#152).
    let back = |points: &[forge_procgen::RibbonPoint], k: usize, up: bool, direction: [f32; 2]| {
        let from = points[k].position;
        let mut reach = 0.0_f32;
        let mut at = k;
        while reach < 200.0 {
            let next = if up {
                at.checked_sub(1)
            } else {
                (at + 1 < points.len()).then_some(at + 1)
            };
            let Some(next) = next else { break };
            let p = &points[next];
            let d = [p.position[0] - from[0], p.position[1] - from[1]];
            let behind = -(d[0] * direction[0] + d[1] * direction[1]);
            let across = (d[0] * direction[1] - d[1] * direction[0]).abs();
            if behind <= reach || across > p.half_width {
                break;
            }
            reach = behind;
            at = next;
        }
        reach.min(200.0)
    };
    let mut mouths: Vec<WaterMouth> = ribbons
        .iter()
        .filter_map(|r| {
            let k = forge_procgen::sea_mouth(&r.points)?;
            Some(WaterMouth {
                white: white(&r.points, k),
                back: back(&r.points, k, true, r.points[k].direction),
                ..mouth(&r.points[k])
            })
        })
        .collect();
    let sea_mouths = mouths.len();
    for r in &ribbons {
        for &[k, last] in &r.lake_runs {
            // The speed the river comes in at: the point before the lake's.
            let (k, last) = (k as usize, last as usize);
            let before = r.points[k.saturating_sub(1)];
            mouths.push(WaterMouth {
                speed: before.speed,
                back: back(&r.points, k, true, r.points[k].direction),
                ..mouth(&r.points[k])
            });
            // And where it runs out (#120): the lake's water drawn into the river, the river's
            // own in a cone back into the lake, so the two meet as one water past the lip.
            if let Some(out) = r.points.get(last + 1) {
                let direction = [-out.direction[0], -out.direction[1]];
                mouths.push(WaterMouth {
                    direction,
                    speed: -out.speed,
                    back: back(&r.points, last + 1, false, direction),
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
        .filter(|r| !r.bars.is_empty() && r.split.is_none())
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
    // The confluences' bars (#119's polish): how many, and the three largest from 25 m back up
    // the river from their head, 10 m over the water, and from straight over them.
    let mut confluence_bars: Vec<forge_procgen::Bar> = ribbons
        .iter()
        .flat_map(|r| r.confluence_bars.iter().copied())
        .collect();
    confluence_bars.sort_by(|a, b| (b.half[0] * b.half[1]).total_cmp(&(a.half[0] * a.half[1])));
    let confluence_views: Vec<String> = confluence_bars
        .iter()
        .take(3)
        .map(|b| {
            let back = 0.6 * b.half[0] + 25.0;
            let at = [
                b.centre[0] - back * b.down[0],
                b.centre[1] - back * b.down[1],
            ];
            let yaw = (-b.down[0]).atan2(-b.down[1]).to_degrees();
            format!(
                "{:.0}m:{:.0},{:.1},{:.0},{yaw:.1},-20  {:.0},{:.1},{:.0},0,-89",
                2.0 * b.half[0],
                at[0] - half_m,
                b.level[0] + 10.0,
                at[1] - half_m,
                b.centre[0] - half_m,
                b.level[1] + 4.0 * b.half[0],
                b.centre[1] - half_m,
            )
        })
        .collect();
    tracing::info!(
        bars = confluence_bars.len(),
        views = %confluence_views.join("  "),
        "the bars past the confluences (--view)"
    );
    // The distributaries (#127): each one's length and width at its mouth, and views of each from
    // 30 m over where it leaves its river, looking down it, and from straight over its middle.
    let branches: Vec<String> = ribbons
        .iter()
        .filter(|r| r.split.is_some())
        .map(|r| {
            let (first, last) = (&r.points[0], &r.points[r.points.len() - 1]);
            let mid = &r.points[r.points.len() / 2];
            let length = r.points.len() as f64 * forge_procgen::RibbonParams::island().step;
            let yaw = (-f64::from(first.direction[0]))
                .atan2(-f64::from(first.direction[1]))
                .to_degrees();
            let back = 40.0;
            let (lx, ly) = (f64::from(last.direction[0]), f64::from(last.direction[1]));
            format!(
                "{length:.0}m x {:.0}m: {:.0},{:.1},{:.0},{yaw:.1},-25  {:.0},{:.1},{:.0},0,-89  \
                 {:.0},{:.1},{:.0},{:.1},-20",
                2.0 * last.half_width,
                f64::from(first.position[0]) - back * f64::from(first.direction[0]) - half_m,
                first.level + 30.0,
                f64::from(first.position[1]) - back * f64::from(first.direction[1]) - half_m,
                f64::from(mid.position[0]) - half_m,
                mid.level + 0.8 * length as f32,
                f64::from(mid.position[1]) - half_m,
                f64::from(last.position[0]) + 50.0 * lx - half_m,
                12.0,
                f64::from(last.position[1]) + 50.0 * ly - half_m,
                lx.atan2(ly).to_degrees(),
            )
        })
        .collect();
    tracing::info!(
        distributaries = branches.len(),
        views = %branches.join("  "),
        "the large rivers' distributaries (--view)"
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
        // How far their water reaches back up their channels (#152): how many fall short of
        // 200 m, and the shortest.
        mouths_back_short = mouths.iter().filter(|m| m.back < 200.0).count(),
        shortest_back_m = %format_args!(
            "{:.0}",
            mouths.iter().map(|m| m.back).fold(f32::INFINITY, f32::min)
        ),
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
    // A view of the fastest water on a 2–4 % slope, where the standing waves are: 12 m upstream
    // and 3 m over the water, looking down the river.
    let rapids_view = points
        .iter()
        .filter(|p| (0.02..=0.04).contains(&p.slope) && p.fade > 0.99)
        .max_by(|a, b| a.speed.total_cmp(&b.speed))
        .map_or_else(String::new, |p| {
            let (dx, dz) = (p.direction[0], p.direction[1]);
            let yaw = (-dx).atan2(-dz).to_degrees();
            format!(
                "{:.0},{:.1},{:.0},{yaw:.1},-14 ({:.1} m/s)",
                p.position[0] - 12.0 * dx,
                p.level + 3.0,
                p.position[1] - 12.0 * dz,
                p.speed
            )
        });
    tracing::info!(
        stones = stones.len(),
        breaking_the_water = stones.iter().filter(|s| s.waterline > 0.0).count(),
        mouths = mouths.len(),
        stone_view = %stone_view,
        rapids_view = %rapids_view,
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

/// The walker's props, the island's last: its capsule and its visor (#196).
const ISLAND_WALKER_PROPS: usize = 2;

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

    /// [`Self::height_at`] with the refined cells too (#196): in one, its fine quads split
    /// along the same diagonal, as `forge_geom::city::refined_heightfield_mesh` draws them. (A
    /// coarse cell beside a refined one is a fan whose triangles are its own two.)
    fn surface_at(&self, x: f64, y: f64) -> f64 {
        let side = self.size - 1;
        let (gx, gy) = (x / self.spacing, y / self.spacing);
        let last = f64::from(side - 1);
        let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
        let cell = cy as u32 * side + cx as u32;
        let Ok(s) = self.detail.cells.binary_search(&cell) else {
            return self.height_at(x, y);
        };
        let k = self.detail.split.max(1) as usize;
        let row = k + 1;
        let heights = &self.detail.heights[s * row * row..(s + 1) * row * row];
        let span = k as f64;
        let (fx, fy) = (
            ((gx - cx) * span).clamp(0.0, span),
            ((gy - cy) * span).clamp(0.0, span),
        );
        let (u, v) = (fx.floor().min(span - 1.0), fy.floor().min(span - 1.0));
        let (tx, ty) = (fx - u, fy - v);
        let at = |u: usize, v: usize| f64::from(heights[v * row + u]);
        let (u, v) = (u as usize, v as usize);
        let (a, b, c, d) = (at(u, v), at(u + 1, v), at(u, v + 1), at(u + 1, v + 1));
        if tx + ty <= 1.0 {
            a + tx * (b - a) + ty * (c - a)
        } else {
            d + (1.0 - tx) * (c - d) + (1.0 - ty) * (b - d)
        }
    }
}

/// The island's field amplified `factor` times finer (stage 5): `forge_procgen::amplify` at
/// each halving of the spacing, over the drainage traced at that spacing.
/// The field [`amplify_ahead`] made, for [`island_amplified`] to take (#201): kept until then
/// only, it is 268 MB at 2 m.
static AMPLIFIED: std::sync::Mutex<Option<(u64, Field2<f32>)>> = std::sync::Mutex::new(None);

fn amplified_key(height: &Field2<f32>, factor: u32, seed: u64) -> u64 {
    height.digest()
        ^ u64::from(height.size)
        ^ u64::from(factor).rotate_left(40)
        ^ seed.rotate_left(20)
}

/// Takes the field [`amplify_ahead`] made for these arguments, or makes it.
fn island_amplified(height: &Field2<f32>, factor: u32, seed: u64, pool: &TaskPool) -> Field2<f32> {
    let key = amplified_key(height, factor, seed);
    let mut ready = AMPLIFIED.lock().expect("the amplified field");
    if ready.as_ref().is_some_and(|(made_for, _)| *made_for == key) {
        return ready.take().expect("the field made ahead").1;
    }
    drop(ready);
    make_island_amplified(height, factor, seed, pool)
}

/// Makes [`island_amplified`]'s field ahead, on the loading thread beside the water, which it
/// does not need (#201). It holds the lock while it works, so a caller waits for it rather than
/// making the field twice.
fn amplify_ahead(height: &Field2<f32>, factor: u32, seed: u64, pool: &TaskPool) {
    let key = amplified_key(height, factor, seed);
    let mut ready = AMPLIFIED.lock().expect("the amplified field");
    if !ready.as_ref().is_some_and(|(made_for, _)| *made_for == key) {
        *ready = Some((key, make_island_amplified(height, factor, seed, pool)));
    }
}

/// [`island_amplified`], made.
fn make_island_amplified(
    height: &Field2<f32>,
    factor: u32,
    seed: u64,
    pool: &TaskPool,
) -> Field2<f32> {
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
            // What the detail adds where it is whole: its root mean square and its largest, a
            // row a task (one thread took a second over the 67 M samples, #201).
            let mut rows = vec![(0.0f64, 0u64, 0.0f32); fine_size];
            pool.par_map_into(&mut rows, 64, |j| {
                let (mut sum, mut count, mut largest) = (0.0f64, 0u64, 0.0f32);
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
                (sum, count, largest)
            });
            let (sum, count, largest) = rows.iter().fold((0.0f64, 0u64, 0.0f32), |a, r| {
                (a.0 + r.0, a.1 + r.1, a.2.max(r.2))
            });
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

/// `--meter-band`: two fractions in 0..1, `LOW,HIGH`, the first below the second.
fn parse_sea_water(text: &str) -> std::result::Result<forge_render::SeaWater, String> {
    forge_render::SeaWater::named(text).ok_or_else(|| {
        let names: Vec<&str> = forge_render::SeaWater::NAMED
            .iter()
            .map(|(n, _)| *n)
            .collect();
        format!("`{text}`: one of {}", names.join(", "))
    })
}

fn parse_pair(text: &str) -> std::result::Result<(f32, f32), String> {
    let parts: Vec<f32> = text
        .split(',')
        .map(|p| p.trim().parse::<f32>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    match parts[..] {
        [low, high] if (0.0..high).contains(&low) && high <= 1.0 => Ok((low, high)),
        _ => Err(format!(
            "`{text}`: two fractions LOW,HIGH with 0 <= LOW < HIGH <= 1"
        )),
    }
}

/// Where the camera starts: the island's first view, the gallery's or the city's, or
/// `--view`.
fn start_camera(args: &Args) -> Result<FlyCamera> {
    let mut camera = if args.lab == Some(lab::LabScene::Fly) {
        // Behind the aeroplane on the runway's threshold; it follows the aeroplane.
        FlyCamera {
            position: Vec3::new(0.0, 4.0, 210.0),
            yaw: 0.0,
            pitch: -0.15,
            speed: 20.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Tug) {
        // In front of the sled, the three lines and most of the ropes in view: the left team's to
        // the left.
        FlyCamera {
            position: Vec3::new(0.0, 2.6, 8.0),
            yaw: 0.0,
            pitch: -0.2,
            speed: 6.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Rocket) {
        // Off the rocket's right on the pad, looking at it; it follows the rocket once flown.
        FlyCamera {
            position: Vec3::new(30.0, 8.0, 5.0),
            yaw: 1.406,
            pitch: -0.01,
            speed: 20.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Bridge) {
        // Beside the gap, a little over the deck, looking across the bridge from its side: the
        // cars come from the left.
        FlyCamera {
            position: Vec3::new(20.0, 7.0, 2.0),
            yaw: 1.5,
            pitch: -0.15,
            speed: 10.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Dominoes) {
        // Over the spiral's outer edge, looking down across it.
        FlyCamera {
            position: Vec3::new(0.0, 6.5, 11.0),
            yaw: 0.0,
            pitch: -0.55,
            speed: 6.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Models) {
        // Before the models' row, or framing the one `--model` names (inside a room model).
        let (position, yaw, pitch) = lab::models::camera();
        FlyCamera {
            position,
            yaw,
            pitch,
            speed: 2.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Room) {
        // In the sharpness room's middle at eye height, looking level at the back wall's targets
        // 8 m away; the board's, 2.5 m away, below them.
        FlyCamera {
            position: Vec3::new(0.0, 1.5, 3.0),
            yaw: 0.0,
            pitch: 0.0,
            speed: 2.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::TankHole) {
        // In front of the tank, right of the gate, a little over the rim: the hole low in the gate
        // and the dry side its jet runs into.
        FlyCamera {
            position: Vec3::new(0.55, 1.3, 1.2),
            yaw: 0.5,
            pitch: -0.25,
            speed: 0.8,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::TankBench) {
        // Over the bench's front right, looking down across the tank at its floor of squares.
        FlyCamera {
            position: Vec3::new(1.0, 1.05, 1.5),
            yaw: 0.58,
            pitch: -0.42,
            speed: 0.8,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::TankBlocks) {
        // In front of the tank's right half, over its rim, looking down at the cube and the posts
        // the wave meets past the gate.
        FlyCamera {
            position: Vec3::new(0.6, 1.3, 0.8),
            yaw: 0.35,
            pitch: -0.55,
            speed: 0.8,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Tank) {
        // In front of the tank and to its right, a little over its rim, looking down into it: the
        // reservoir behind the gate on the left.
        FlyCamera {
            position: Vec3::new(0.45, 1.4, 1.6),
            yaw: 0.28,
            pitch: -0.22,
            speed: 0.8,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Flood) {
        // Over the basin's lower corner, looking up it at the gate and the reservoir.
        FlyCamera {
            position: Vec3::new(12.0, 9.0, 16.0),
            yaw: 0.9,
            pitch: -0.34,
            speed: 10.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Creatures) {
        // Before the creatures, at a man's height, the mannequins behind the dogs.
        FlyCamera {
            position: Vec3::new(0.0, 1.4, 4.4),
            yaw: 0.0,
            pitch: -0.1,
            speed: 6.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Course) {
        // Beside the course and over it, looking down across both lanes: the ramps nearer.
        FlyCamera {
            position: Vec3::new(3.3, 2.1, 0.6),
            yaw: std::f32::consts::FRAC_PI_2,
            pitch: -0.52,
            speed: 6.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Yard) {
        // Over the end of the car's snow, looking down across its ruts to the dogs' beds (#186).
        FlyCamera {
            position: Vec3::new(5.6, 2.2, 2.6),
            yaw: 60f32.to_radians(),
            pitch: -40f32.to_radians(),
            speed: 6.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Flyer) {
        // Outside the gulls' circuit and under it, looking across it: the near ones pass close.
        FlyCamera {
            position: Vec3::new(0.0, 5.0, 45.0),
            yaw: 0.0,
            pitch: 0.05,
            speed: 10.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Break) {
        // In front of the wall and to its left, clear of the gantry's post: the wall's face, the
        // column behind it, the ball held back at the right edge.
        FlyCamera {
            position: Vec3::new(-6.5, 2.4, 7.5),
            yaw: -0.71,
            pitch: -0.1,
            speed: 6.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Drive) {
        // Behind the car and to its right, the track ahead along −z.
        FlyCamera {
            position: Vec3::new(4.0, 3.0, 8.0),
            yaw: 0.35,
            pitch: -0.15,
            speed: 10.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Materials) {
        // Behind the walker's start and to its left, high enough to see the patches along its
        // way, their ramps beyond and the ice at the end (#203).
        FlyCamera {
            position: Vec3::new(-9.0, 7.0, -24.0),
            yaw: -2.79,
            pitch: -0.21,
            speed: 8.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Walk) {
        // Behind the player, looking along −z at the ramps, a little down; it follows the
        // player from the first frame.
        FlyCamera {
            position: Vec3::new(0.0, 3.0, 6.0),
            yaw: 0.0,
            pitch: -0.25,
            speed: 8.0,
            ..FlyCamera::default()
        }
    } else if args.lab == Some(lab::LabScene::Sea) {
        // Over the jetty, looking out past its end at the water where things fell, the boat on
        // the left.
        FlyCamera {
            position: Vec3::new(-2.0, 6.5, -4.0),
            yaw: -0.42,
            pitch: -0.2,
            speed: 8.0,
            ..FlyCamera::default()
        }
    } else if args.lab.is_some() {
        // South-east of the pyramid, a little above its top, looking at it.
        FlyCamera {
            position: Vec3::new(13.0, 7.5, 17.0),
            yaw: 0.65,
            pitch: -0.22,
            speed: 8.0,
            ..FlyCamera::default()
        }
    } else if args.island.is_some() {
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
    ctx: &Setup,
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
    let beach = island_beach(args);
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

/// The island's southern beach on its middle line (x = 0): the scene's z of the southmost sample
/// there over 1 m, where the land begins (the camera starts 150 m off it; a bare `--walker`
/// stands on it, #196).
fn island_beach(args: &Args) -> f64 {
    let height = island_heights(args);
    let n = height.size;
    let half = height.extent() * 0.5;
    (0..n)
        .rev()
        .find(|&j| height.get(n / 2, j) > 1.0)
        .map_or(0.0, |j| f64::from(j) * height.spacing - half)
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
    // The movers' barrel, log and crate last (#79, #177), after the rocks.
    if args.movers > 0 {
        props.push(barrel_prop());
        props.extend(afloat::props());
    }
    // The walker's capsule and visor after them (#196).
    props.extend(lab::walk::player_props());
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
/// The air near the water, a share of the sea's wind at 10 m: the spray drifts in it.
const SPRAY_WIND: f32 = 0.15;

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
    ctx: &Setup,
    args: &Args,
    cooked: Cooked,
    camera: &FlyCamera,
) -> Result<(MeshletScene, island_sand::SandWindow)> {
    let start = Instant::now();
    let props = island_props(args);
    let streamed = args.stream_pool > 0;
    let (meshes, cook_ms) = (cooked.meshes, cooked.ms);
    let mut builder = MeshletSceneBuilder::new();
    let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    // The layers from the field: a texel every 4 m over the 16 km (stage 6's first rule), and
    // the rock sites, painted on the CPU: on the loading thread first (#201).
    let height = island_heights(args);
    let extent = height.extent() as f32;
    let texels = ISLAND_TEXELS;
    let IslandWater {
        ribbons,
        channels,
        lakes: lake_waters,
    } = island_water(&height);
    let painted = island_layers(args);
    let layers = painted.layers.clone();
    let sites = &painted.sites;
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
    // The beach's sand round the walker (#197): a ground window the tiles leave to it, a mesh
    // of a vertex per point of its layer, raised by the ground and the layer and shaded by the
    // ground's own row where it stands.
    let ground_row = materials.of(&props[0].name);
    let window_mesh = builder.add_displaced_mesh(
        island_sand::SandWindow::cooked_mesh(),
        island_sand::SandWindow::height_field(),
    );
    builder.set_mesh_material(window_mesh, ground_row);
    builder.set_ground_window_mesh(window_mesh);
    for &tile in tile_ids {
        builder.set_windowed_ground(tile);
    }
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
    let banked = island_bank_stones(&height, &ribbons, &channels);
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
        // The props after the tiles and the sea, less the movers' barrel, log and crate and the
        // walker's two.
        let movers_props = if args.movers > 0 { 3 } else { 0 };
        (
            ids[tiles + 1..ids.len() - ISLAND_WALKER_PROPS - movers_props].to_vec(),
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
    // The movers (#79), the table's last instances: their transforms come every frame. The
    // barrels', then the walker's capsule and visor (#196).
    let mut movers = Vec::new();
    if args.movers > 0 {
        let [barrel, log, crate_] =
            [3, 2, 1].map(|from_end| ids[ids.len() - ISLAND_WALKER_PROPS - from_end]);
        let [barrels, logs, crates] = Barrels::movers(args.movers);
        movers.extend([(barrel, barrels), (log, logs), (crate_, crates)]);
    }
    movers.extend([2, 1].map(|from_end| (ids[ids.len() - from_end], 1)));
    // Then the sand window's (#197).
    movers.push((window_mesh, 1));
    builder.reserve_movers(&movers);
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
    let sand = island_sand::SandWindow::new(island_drawn(args), Arc::new(layers));
    Ok((scene, sand))
}

/// The city: the terrain and the twenty props cooked (or loaded), the terrain placed once
/// at the origin and `args.instances` props placed over it by the GPU.
fn build_city(
    ctx: &Setup,
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
fn build_gallery(ctx: &Setup, args: &Args, cooked: Cooked) -> Result<(MeshletScene, Vec<Placed>)> {
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

/// Whether the run anti-aliases with DLAA where the device has it (D-045): an interactive run
/// unless `--no-dlaa` or `--no-taa`, a scripted one (`--frames`) only with `--dlaa`. DLAA's
/// images differ by a code or two from run to run, and the captures' checks want them to the bit.
fn wants_dlaa(args: &Args) -> bool {
    !args.no_dlaa && !args.no_taa && (args.dlaa || args.frames.is_none())
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

/// The `physics-lab` binary (#136): the same demo with one of the lab's scenes (`--lab`, `drop`
/// unless given).
pub fn main_lab() -> Result<()> {
    let matches = Args::command()
        .name("physics-lab")
        .about("The physics lab: rigid bodies through Jolt, one test scene at a time")
        .get_matches();
    let mut args = Args::from_arg_matches(&matches)?;
    args.lab.get_or_insert(lab::LabScene::Drop);
    run(args, "forge physics-lab")
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
            confluence_scour: island.confluence_scour.filter(|_| !args.no_scour),
            confluence_bars: island.confluence_bars.filter(|_| !args.no_confluence_bars),
            distributaries: island.distributaries.filter(|_| !args.no_distributaries),
            ..island
        })
        .expect("the rivers' parameters, set once");
    SILLS.set(!args.no_sills).expect("the sills, set once");
    TEXTURES
        .set(args.textures)
        .expect("the textures' mode, set once");
    lab::models::FOCUS
        .set(args.model.clone())
        .expect("the models scene's model, set once");
    let config = AppConfig {
        title: title.into(),
        vsync: args.vsync,
        validate: args.validate,
        frame_limit: args.frames,
        capture: args.capture.clone().map(|p| (p, args.capture_frame)),
        capture_every: args.capture_every,
        overlay: if args.overlay { Some(true) } else { None },
        force_fallback: args.force_fallback,
        // The Vulkan loader through Streamline only where DLAA may run, in a `dlss` build.
        streamline: cfg!(feature = "dlss") && wants_dlaa(&args),
        hdr: args.hdr,
        hdr_stops: args.hdr_stops,
        hdr_ui_white: args.hdr_ui_white,
        ssaa: args.ssaa,
        width: args.width,
        height: args.height,
        ..AppConfig::default()
    };
    // The props cook (or load from the cache) behind the loading screen (issue #25).
    forge_app::run_loading(config, move || {
        let cooked = cook(&args);
        if args.island.is_some() {
            warm_island(&args);
        }
        let finish: Finish<Gallery> = Box::new(move |ctx| Gallery::new(ctx, args, cooked, title));
        Ok(finish)
    })
}

/// The island's CPU work, done on the loading thread while the loading screen draws (#201): its
/// heights, its water, then its drawn ground, its rivers' stones and its layers side by side. Each is made once a
/// process, so the finishing step on the main thread, which needs the device, finds them made;
/// before, it made them there behind a frozen screen (about 13 s of 15 with a warm cache).
fn warm_island(args: &Args) {
    let start = Instant::now();
    let height = island_heights(args);
    let factor = island_factor(args);
    std::thread::scope(|ahead| {
        // The drawn ground's amplified detail needs the heights alone: beside the water.
        if factor > 1 && args.island_detail > 0.0 {
            ahead.spawn(|| {
                amplify_ahead(
                    &height,
                    factor,
                    args.island.unwrap_or(7),
                    &TaskPool::client(),
                );
            });
        }
        let water = island_water(&height);
        std::thread::scope(|threads| {
            threads.spawn(|| island_drawn(args));
            threads.spawn(|| {
                island_stones(&height, &water.ribbons, &water.channels);
                island_bank_stones(&height, &water.ribbons, &water.channels);
                island_sand::SandWindow::cooked_mesh();
            });
            island_layers(args);
        });
    });
    tracing::info!(
        ms = start.elapsed().as_millis(),
        "the island's heights, water, drawn ground and layers, behind the loading screen (#201)"
    );
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

/// Texels a side of the island's layer map: one every 4 m over the 16 km.
const ISLAND_TEXELS: u32 = 4096;

/// The island's ground layers and where its loose rocks lie (#130): what the CPU paints for
/// `build_island`, in `make_island_layers`.
struct IslandLayers {
    layers: Field2<u8>,
    /// The rock sites' map, its numbers and the milliseconds it took (`--no-rock-sites`: none).
    sites: Option<(Field2<u8>, forge_procgen::RockSiteStats, u128)>,
}

/// [`IslandLayers`], made once a process for the island and the arguments that change it: the
/// loading thread makes it (#201), so the finishing step on the main thread finds it.
fn island_layers(args: &Args) -> Arc<IslandLayers> {
    static MADE: std::sync::Mutex<Option<(u64, Arc<IslandLayers>)>> = std::sync::Mutex::new(None);
    let height = island_heights(args);
    let flags = [
        args.water(),
        args.no_salt,
        args.no_beach_types,
        args.no_rock_types,
        args.no_rock_sites,
    ];
    let key = flags
        .iter()
        .enumerate()
        .fold(height.digest() ^ u64::from(height.size), |key, (k, &on)| {
            key ^ (u64::from(on) << (56 + k))
        });
    let mut made = MADE.lock().expect("the island's layers");
    if let Some((made_for, layers)) = made.as_ref()
        && *made_for == key
    {
        return layers.clone();
    }
    let layers = Arc::new(make_island_layers(args, &height, ISLAND_TEXELS));
    *made = Some((key, layers.clone()));
    layers
}

/// [`island_layers`], made: the slope rule's layers, then the moisture, the salt water's sand,
/// the beaches, the rivers' and lakes' layers, the valleys' ground and the geology, and the rock
/// sites from them.
fn make_island_layers(args: &Args, height: &Field2<f32>, texels: u32) -> IslandLayers {
    let height = height.clone();
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
    // The salt water keeps the grass off the banks beside it (#199, the owner's note: "grass
    // don't like the salted water much"): sand up to 5 m over the sea at the water, falling to
    // the beach's top 40 m from it, on the sea's slopes and the rivers' reaches it fills.
    if !args.no_salt {
        let salt_start = Instant::now();
        let salted = forge_procgen::paint_salt(
            &mut layers,
            &|x, y| channels.height_at(&height, x, y),
            &[
                island_layer::GRASS,
                island_layer::DRY_GRASS,
                island_layer::LUSH_GRASS,
            ],
            island_layer::SEABED,
            island_layer::SAND,
            f64::from(SAND_BELOW),
            &forge_procgen::SaltRule::default(),
        );
        tracing::info!(
            salted,
            ms = salt_start.elapsed().as_millis(),
            "the salt water's sand (#199, --no-salt)"
        );
    }
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
    // The bars the confluences lay along the bank past their corner (#119's polish): the sand of
    // the mouths' bars (the deltas' sand, made to lie under water, reads as a dark stain in the
    // sun).
    let confluence_bar_texels = if args.water() {
        forge_procgen::paint_confluence_bars(
            &mut layers,
            &ribbons,
            &|x, y| channels.height_at(&height, x, y),
            island_layer::SAND,
        )
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
        confluence_bar_texels,
        dry_texels = dried,
        lush_texels = greened,
        ms = rivers_start.elapsed().as_millis(),
        "island moisture, rivers and lakes"
    );
    // The rock by the island's geology (D-042, #129), after the rules that read the rock: the
    // hills' granite, the low ground's limestone, and karst on the limestone's dry ground, grus on
    // the granite's gentle ground near its bare rock (#135).
    if !args.no_rock_types {
        let geology_start = Instant::now();
        let rocks = forge_procgen::paint_geology(
            &mut layers,
            &height,
            forge_procgen::GeologyLayers {
                rock: island_layer::ROCK,
                limestone: island_layer::LIMESTONE,
                grass: island_layer::GRASS,
                dry_grass: island_layer::DRY_GRASS,
                karst: island_layer::KARST,
                grus: island_layer::GRUS,
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
            island_layer::GRUS,
        ]
        .iter()
        .filter_map(|&l| view(l))
        .collect();
        tracing::info!(
            granite_texels = rocks.granite,
            limestone_texels = rocks.limestone,
            karst_texels = rocks.karst,
            grus_texels = rocks.grus,
            ms = geology_start.elapsed().as_millis(),
            views = %views.join("  "),
            "the island's rocks: granite, limestone, karst, grus (D-042, #129, #135, --view)"
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
    IslandLayers { layers, sites }
}

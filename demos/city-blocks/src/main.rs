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
//! threshold, T TAA, Tab wireframe, G tone curve, Esc quit.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_app::{AppConfig, Context, Demo, FlyCamera, FrameInfo, Input};
use forge_core::material::{
    Material, MaterialId, MaterialTable, RenderLayer, ShadingClass, TextureId,
};
use forge_geom::MeshletMesh;
use forge_geom::cache::cook_cached;
use forge_geom::city::{PropKind, PropSpec, Terrain, city_props};
use forge_render::material::TextureSet;
use forge_render::meshlet::{DrawParams, MeshId};
use forge_render::placement::{self, CityLayout, CityMeshes, Ground};
use forge_render::textures::{self, TextureData};
use forge_render::{
    Atmosphere, AtmosphereParams, Bloom, CullCamera, CullFlags, FrameStats, GroundSky,
    MeshletRenderer, MeshletScene, MeshletSceneBuilder, Residency, SkyParams, StreamingConfig,
    StreamingStats, SwRaster, Taa, Tonemap, exposure_from_ev100,
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
    /// Show the twenty props side by side instead of the city.
    #[arg(long)]
    gallery: bool,
    /// Instances placed over the terrain (the city takes about 12 k, the hills the rest).
    #[arg(long, default_value_t = 1_000_000)]
    instances: u32,
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
    /// The sun's elevation over the horizon, degrees (63.4: the renderer's default sun).
    #[arg(long, default_value_t = 63.4)]
    sun_elevation: f32,
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
    fn new(ctx: &mut Context, args: Args) -> Result<Self> {
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
        // The sun at `--sun-elevation`, from the default sun's azimuth, through the air.
        let atmosphere_params = AtmosphereParams::earth();
        let elevation = args.sun_elevation.to_radians();
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
        let atmosphere = Atmosphere::new(&ctx.device, &ctx.shaders, atmosphere_params)?;
        let sky = GroundSky::new(&ctx.device, &ctx.shaders)?;
        let (scene, placed) = if args.gallery {
            build_gallery(ctx, &args)?
        } else {
            (build_city(ctx, &args)?, Vec::new())
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
        let mut camera = if args.gallery {
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

    fn cull_camera(&self, aspect: f32) -> CullCamera {
        CullCamera::new(
            self.camera.view(),
            self.camera.projection(aspect),
            self.camera.position,
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
                "drawn through {}: {} instances, {:.0} k + {:.0} k clusters, {:.2} M triangles, {:.0} k occluded{}; LOD {} at {:.2} px",
                self.renderer.path().name(),
                last.instances_visible,
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
        let camera = self.cull_camera(ctx.aspect());
        let extent = ctx.extent();
        let exposure = exposure_from_ev100(self.args.ev100);
        // Draw jittered into TAA's HDR target, cull with the unjittered camera, resolve
        // through the history and the tone curve into the swapchain.
        let taa_frame = self.taa.begin(
            &mut frame.graph,
            self.camera.projection(ctx.aspect()),
            camera.view_proj,
            exposure,
        );
        let targets = self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: taa_frame.jittered_projection * self.camera.view(),
                cull: camera,
                lod_threshold_px: self.args.lod_error,
                draw_jitter: taa_frame.jitter
                    / glam::Vec2::new(extent.width as f32, extent.height as f32),
                flags: self.flags,
                extent,
                wireframe: self.wireframe,
                exposure,
                sw_raster: self.args.sw_raster,
                sw_raster_area: self.args.sw_raster_area,
            },
        )?;
        // The ground of the city is the surface of an Earth-sized planet: the camera in its
        // frame, in km. The sky fills what the resolve leaves and hazes the rest (issue #43).
        let view_km = Vec3::new(
            self.camera.position.x * 1e-3,
            self.atmosphere.params.bottom_radius + self.camera.position.y.max(1.0) * 1e-3,
            self.camera.position.z * 1e-3,
        );
        let air = self.atmosphere.frame(&mut frame.graph, frame.slot, view_km);
        self.renderer.resolve(
            &mut frame.graph,
            frame.slot,
            targets,
            taa_frame.color,
            extent,
            None,
        );
        self.sky.draw(
            &mut frame.graph,
            frame.slot,
            &air,
            SkyParams {
                view_proj: taa_frame.jittered_projection * self.camera.view(),
                camera: self.camera.position,
                sun_dir: self.renderer.sun_dir,
                sun_angular_radius: forge_render::starfield::SUN_ANGULAR_RADIUS_1AU,
                luminance_scale: self.renderer.sun_illuminance * exposure,
                aerial_far_km: 8.0,
            },
            targets.depth,
            taa_frame.color,
            extent,
        );
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
            "forge city-blocks | {} instances, {:.1} M triangles, {:.1} M clusters | {}: drawn {:.0} k instances, {:.0} k + {:.0} k clusters ({:.0} k in software; {:.0} k work items, {:.0} k roots), {:.2} M tris | GPU {:.2} ms, frame p50 {p50:.2} p99 {p99:.2} ms",
            self.scene.instance_count,
            self.scene.total_triangles as f64 / 1e6,
            self.scene.instance_meshlets() as f64 / 1e6,
            self.renderer.path().name(),
            mean(|s| s.instances_visible) / 1e3,
            mean(|s| s.meshlets_pass1) / 1e3,
            mean(|s| s.meshlets_pass2) / 1e3,
            mean(|s| s.sw_clusters) / 1e3,
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
        let glass = add(
            "dark glass",
            textured(
                concrete,
                [0.18, 0.21, 0.26],
                [0.15, 0.17, 0.2],
                6.0,
                90.0,
                0.35,
            ),
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

/// The city: the terrain and the twenty props cooked (or loaded), the terrain placed once
/// at the origin and `args.instances` props placed over it by the GPU.
fn build_city(ctx: &Context, args: &Args) -> Result<MeshletScene> {
    let start = Instant::now();
    let terrain = Terrain::city();
    let mut props = city_props();
    props.push(PropSpec {
        name: "terrain".to_owned(),
        kind: PropKind::Terrain(terrain.clone()),
    });
    let streamed = args.stream_pool > 0;
    let (meshes, cook_ms) = cook_props(&props, args.recook, !streamed);
    let mut builder = MeshletSceneBuilder::new();
    let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    let layout = CityLayout::city(args.instances);
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
    let scene = builder.build_with(&ctx.device, residency)?;
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
fn build_gallery(ctx: &Context, args: &Args) -> Result<(MeshletScene, Vec<Placed>)> {
    let start = Instant::now();
    let props = city_props();
    let (meshes, total_ms) = cook_props(&props, args.recook, true);
    let mut builder = MeshletSceneBuilder::new();
    let mut placed = Vec::with_capacity(props.len());
    let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
    CityMaterials::new(&ctx.device)?.apply(&mut builder, &props, &ids);
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
    let scene = builder.build(&ctx.device)?;
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
    forge_app::run(config, move |ctx| Gallery::new(ctx, args))
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

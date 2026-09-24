//! `city-blocks` — the Phase 1 closing demo (issue #13), built in steps. This first step
//! (issue #34) is the prop gallery: the twenty procedural props of the city set, 0.5 to 3 M
//! triangles each, cooked into cluster DAGs once and cached on disk (`mesh-cache/`), drawn
//! side by side through the GPU-driven meshlet renderer.
//!
//! Controls: WASD/QE move, Shift fast, right mouse look, L cluster LOD, K LOD colours, M
//! cluster colours, O occlusion, R software rasteriser, H show what it drew, [ / ] LOD
//! threshold, Tab wireframe, G tone curve, Esc quit.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_app::{AppConfig, Context, Demo, FlyCamera, FrameInfo, Input};
use forge_app::{TransientDesc, vk};
use forge_geom::MeshletMesh;
use forge_geom::cache::cook_cached;
use forge_geom::city::{PropSpec, city_props};
use forge_render::meshlet::DrawParams;
use forge_render::{
    CullCamera, CullFlags, Display, FrameStats, HDR_FORMAT, MeshletRenderer, MeshletScene,
    MeshletSceneBuilder, SwRaster, Tonemap, exposure_from_ev100,
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
    /// Start framed on this prop (its name in the log, e.g. `fountain`).
    #[arg(long)]
    focus: Option<String>,
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
    display: Display,
    tonemap: Tonemap,
    scene: MeshletScene,
    camera: FlyCamera,
    flags: CullFlags,
    wireframe: bool,
    frame: u64,
    stats: Vec<FrameStats>,
    gpu_ms: Vec<f64>,
    title_updates: u32,
}

/// Metres between the centres of neighbouring props in the gallery.
const SPACING: f32 = 60.0;
/// Props per row of the gallery.
const COLUMNS: u32 = 5;

impl Gallery {
    fn new(ctx: &mut Context, args: Args) -> Result<Self> {
        let mut renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, ctx.extent())?;
        let display = Display::new(&ctx.device, &ctx.shaders, ctx.swapchain.format())?;
        let (scene, placed) = build_gallery(ctx, &args)?;
        let mut flags = CullFlags(CullFlags::CONE | CullFlags::FRUSTUM);
        if !args.no_lod {
            flags.0 |= CullFlags::LOD;
        } else {
            renderer.reserve_visible(scene.finest_clusters);
        }
        if !args.no_occlusion {
            flags.0 |= CullFlags::OCCLUSION;
        }
        let mut camera = FlyCamera {
            position: Vec3::new(0.0, 70.0, 230.0),
            pitch: -0.3,
            speed: 40.0,
            ..FlyCamera::default()
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
            display,
            scene,
            camera,
            flags,
            wireframe: false,
            frame: 0,
            stats: Vec::new(),
            gpu_ms: Vec::new(),
            title_updates: 0,
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
            _ => {}
        }
    }

    fn update(&mut self, _ctx: &mut Context, input: &Input, dt: f32) {
        if self.args.orbit {
            // Deterministic per frame (not per second) so captures at a frame index match.
            let angle = self.frame as f32 * 0.004;
            let radius = 230.0 - (self.frame as f32 * 0.1).min(120.0);
            self.camera.position = Vec3::new(angle.sin() * radius, 45.0, angle.cos() * radius);
            self.camera.yaw = angle;
            self.camera.pitch = -0.22;
        } else {
            self.camera.update(input, dt);
        }
        self.frame += 1;
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        if let Some(stats) = self.renderer.begin_frame(frame.slot, &self.scene)? {
            self.stats.push(stats);
            if let Some(ms) = frame.slot.previous_gpu_ms {
                self.gpu_ms.push(ms);
            }
        }
        if let Some(last) = self.stats.last() {
            ctx.profile.counter(format!(
                "drawn through {}: {} props, {:.0} k + {:.0} k clusters, {:.2} M triangles, {:.0} k occluded{}; LOD {} at {:.2} px",
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
        let color = frame.graph.transient(TransientDesc {
            name: "gallery color",
            width: extent.width,
            height: extent.height,
            format: HDR_FORMAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let targets = self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: camera.view_proj,
                cull: camera,
                lod_threshold_px: self.args.lod_error,
                draw_jitter: glam::Vec2::ZERO,
                flags: self.flags,
                extent,
                wireframe: self.wireframe,
                exposure: exposure_from_ev100(self.args.ev100),
                sw_raster: self.args.sw_raster,
                sw_raster_area: self.args.sw_raster_area,
            },
        )?;
        // A pale sky behind the props (the terrain and the sky come with issue #35).
        self.renderer.resolve(
            &mut frame.graph,
            frame.slot,
            targets,
            color,
            extent,
            Some([0.55, 0.62, 0.72, 1.0]),
        );
        self.display
            .draw(&mut frame.graph, color, frame.target, extent, self.tonemap);
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
        let title = format!(
            "forge city-blocks | {} props, {:.1} M triangles, {:.1} M clusters | {}: drawn {:.0} k + {:.0} k clusters ({:.0} k in software), {:.2} M tris | GPU {:.2} ms",
            self.scene.instance_count,
            self.scene.total_triangles as f64 / 1e6,
            self.scene.instance_meshlets() as f64 / 1e6,
            self.renderer.path().name(),
            mean(|s| s.meshlets_pass1) / 1e3,
            mean(|s| s.meshlets_pass2) / 1e3,
            mean(|s| s.sw_clusters) / 1e3,
            mean(|s| s.triangles) / 1e6,
            gpu,
        );
        self.stats.clear();
        self.gpu_ms.clear();
        self.title_updates += 1;
        if self.title_updates.is_multiple_of(4) {
            tracing::info!("{title}");
        }
        Some(title)
    }
}

/// Cooks (or loads) every prop of the city set in parallel and lays one of each out on a
/// grid, `SPACING` metres apart.
fn build_gallery(ctx: &Context, args: &Args) -> Result<(MeshletScene, Vec<Placed>)> {
    let start = Instant::now();
    let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
    let cache = root.join("mesh-cache");
    if args.recook {
        // Stale files would still match their keys: remove this set's before cooking.
        for spec in city_props() {
            let key = forge_geom::cache::key(&spec.key_text());
            let _ = std::fs::remove_file(forge_geom::cache::path(&cache, &spec.name, key));
        }
    }
    let props = city_props();
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
                    || spec.generate(),
                );
                if let Err(error) = stored {
                    tracing::warn!(prop = %spec.name, %error, "cooked mesh not cached");
                }
                *slot = Some((done.mesh, done.from_cache, done.ms));
            });
        }
    });
    let mut builder = MeshletSceneBuilder::new();
    let mut placed = Vec::with_capacity(props.len());
    let mut total_ms = 0.0;
    for (i, (spec, done)) in props.iter().zip(&cooked).enumerate() {
        let (mesh, from_cache, ms) = done.as_ref().expect("prop cooked");
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
        let id = builder.add_mesh(mesh);
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
        ..AppConfig::default()
    };
    forge_app::run(config, move |ctx| Gallery::new(ctx, args))
}

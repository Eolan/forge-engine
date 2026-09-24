//! `asteroids` — the ballad. A scripted flight through a dense asteroid field, the engine's
//! living showcase: whatever the renderer can do at this point, it does here, with profiling.
//!
//! Phase 0: several procedural asteroid meshes, thousands of instances, task/mesh shaders,
//! two-pass occlusion culling, a procedural starfield, a spline camera path.
//!
//! Controls: P pause/resume the path (right mouse look + WASD to fly freely while paused),
//! T temporal anti-aliasing, M meshlet colours, O occlusion, Tab wireframe, Esc quit.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_app::{AppConfig, Context, Demo, FlyCamera, FrameInfo, Input};
use forge_core::{Seed, SplitMix64};
use forge_geom::{MeshletMesh, procedural};
use forge_render::meshlet::DrawParams;
use forge_render::{
    ColorLoad, CullCamera, CullFlags, FrameStats, HDR_FORMAT, MeshletRenderer, MeshletScene,
    MeshletSceneBuilder, Starfield, Taa,
};
use forge_task::TaskPool;
use glam::{Mat4, Quat, Vec3};
use winit::keyboard::KeyCode;

#[derive(Parser, Debug, Clone)]
#[command(about = "The asteroid ballad")]
struct Args {
    /// Number of asteroids in the field.
    #[arg(long, default_value_t = 3000)]
    count: u32,
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
    taa: Taa,
    taa_enabled: bool,
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
    fn new(ctx: &mut Context, args: Args) -> Result<Self> {
        if !ctx.device.features().mesh_shader {
            anyhow::bail!(
                "{} has no mesh shader support (VK_EXT_mesh_shader)",
                ctx.device.name()
            );
        }
        // The scene is drawn into the TAA's HDR target; the swapchain only receives the resolve.
        let renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, HDR_FORMAT, ctx.extent())?;
        let mut starfield = Starfield::new(&ctx.device, &ctx.shaders, HDR_FORMAT)?;
        // A large planet low on the horizon, lit from the side by the sun.
        starfield.planet_angle = args.planet_angle.to_radians();
        starfield.planet_dir = args.planet_dir.normalize_or(Vec3::NEG_Z);
        let mut renderer = renderer;
        renderer.sun_dir = args.sun_dir.normalize_or(Vec3::Y);
        let taa = Taa::new(
            &ctx.device,
            &ctx.shaders,
            ctx.extent(),
            ctx.swapchain.format(),
        )?;
        let (scene, path) = build_field(ctx, &args)?;
        let camera = FlyCamera {
            speed: 40.0,
            ..FlyCamera::default()
        };
        let taa_enabled = !args.no_taa;
        let mut flags = CullFlags::DEFAULT;
        if args.no_occlusion {
            flags.toggle(CullFlags::OCCLUSION);
        }
        if args.no_cone {
            flags.toggle(CullFlags::CONE);
        }
        if args.no_lod {
            flags.toggle(CullFlags::LOD);
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
        let mut taa = taa;
        taa.blend = args.taa_blend;
        Ok(Self {
            args,
            renderer,
            starfield,
            taa,
            taa_enabled,
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

impl Demo for Ballad {
    fn resized(&mut self, ctx: &mut Context) -> Result<()> {
        self.renderer.resize(ctx.extent())?;
        self.taa.resize(ctx.extent())?;
        Ok(())
    }

    fn key_pressed(&mut self, _ctx: &mut Context, code: KeyCode) {
        match code {
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
            KeyCode::BracketLeft => self.args.lod_error = (self.args.lod_error * 0.5).max(0.125),
            KeyCode::BracketRight => self.args.lod_error = (self.args.lod_error * 2.0).min(16.0),
            KeyCode::Tab => self.wireframe = !self.wireframe,
            _ => {}
        }
    }

    fn update(&mut self, _ctx: &mut Context, input: &Input, dt: f32) {
        let now = Instant::now();
        self.frame_ms
            .push((now - self.last_frame).as_secs_f64() * 1e3);
        self.last_frame = now;
        if self.paused {
            self.camera.update(input, dt);
            return;
        }
        let step = if self.args.fixed_step {
            1.0 / 120.0
        } else {
            dt
        };
        self.path_t += step / self.args.duration;
        let position = self.path.sample(self.path_t);
        let ahead = self.path.sample(self.path_t + 0.004);
        let forward = (ahead - position).normalize_or_zero();
        // Look along the tangent with a gentle roll into the turns.
        let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
        self.camera.position = position;
        self.camera.yaw = -flat.x.atan2(-flat.z);
        self.camera.pitch = forward.y.asin().clamp(-1.2, 1.2);
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        let cpu_start = Instant::now();
        if let Some(stats) = self.renderer.take_stats(frame.slot) {
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
                "drawn: {} instances, {:.0} k + {:.0} k meshlets, {:.2} M triangles, {:.0} k occluded",
                last.instances_visible,
                f64::from(last.meshlets_pass1) / 1e3,
                f64::from(last.meshlets_pass2) / 1e3,
                f64::from(last.triangles) / 1e6,
                f64::from(last.occluded) / 1e3
            ));
            ctx.profile.counter(format!(
                "LOD {} at {:.2} px: mean level {:.2} of the drawn clusters; {} clusters in the DAG tables",
                if self.flags.has(CullFlags::LOD) { "on" } else { "off" },
                self.args.lod_error,
                f64::from(last.lod_level_sum) / f64::from((last.meshlets_pass1 + last.meshlets_pass2).max(1)),
                self.scene.meshlet_count
            ));
            ctx.profile.counter(format!(
                "TAA {}   occlusion {}   cone {}   {}",
                if self.taa_enabled { "on" } else { "off" },
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
        let cull = self.cull_camera(ctx.aspect());
        let extent = ctx.extent();
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
                    "{} t={:.9} pos={:?} yaw={} pitch={} taa={} flags={} prev={:?}",
                    ctx.frames_rendered,
                    self.path_t,
                    self.camera.position.to_array(),
                    self.camera.yaw,
                    self.camera.pitch,
                    self.taa.frame_index(),
                    self.flags.0,
                    &prev[..8]
                );
            }
        }
        // Draw jittered into the HDR target; cull with the unjittered camera. The rocks go
        // first, then the sky fills the pixels they left (depth-tested), then the resolve.
        self.taa.enabled = self.taa_enabled;
        let taa_frame = self.taa.begin(
            &mut frame.graph,
            self.camera.projection(ctx.aspect()),
            cull.view_proj,
        );
        let draw_view_proj = taa_frame.jittered_projection * self.camera.view();
        let depth = self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: draw_view_proj,
                cull,
                lod_threshold_px: self.args.lod_error,
                draw_jitter: taa_frame.jitter
                    / glam::Vec2::new(extent.width as f32, extent.height as f32),
                flags: self.flags,
                color: taa_frame.color,
                extent,
                color_load: ColorLoad::DontCare,
                wireframe: self.wireframe,
            },
        )?;
        self.starfield.draw(
            &mut frame.graph,
            taa_frame.color,
            depth,
            extent,
            draw_view_proj,
            self.renderer.sun_dir,
        );
        self.taa
            .resolve(&mut frame.graph, &taa_frame, depth, frame.target);
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
            "forge asteroids | {} asteroids, {} meshes, {:.1} M meshlets, {:.0} M tris | drawn {:.0} k + {:.0} k meshlets, {:.2} M tris | GPU {:.2} ms  CPU {:.2} ms  frame p50 {:.2} p99 {:.2} ms | {}{}{}{}{}",
            self.scene.instance_count,
            self.scene.mesh_count,
            self.scene.instance_meshlets() as f64 / 1e6,
            self.scene.total_triangles as f64 / 1e6,
            mean(|s| s.meshlets_pass1) / 1e3,
            mean(|s| s.meshlets_pass2) / 1e3,
            mean(|s| s.triangles) / 1e6,
            gpu,
            cpu,
            p(0.5),
            p(0.99),
            if self.paused { "[P paused] " } else { "" },
            if self.taa_enabled { "[T taa] " } else { "" },
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

/// The field: a few distinct asteroid meshes, thousands of instances clustered along a
/// curved belt, and a camera path weaving through it.
fn build_field(ctx: &Context, args: &Args) -> Result<(MeshletScene, Path)> {
    let start = Instant::now();
    let pool = TaskPool::client();
    // Mesh recipes: (segments per face, radius, roughness). Built in parallel.
    let recipes: [(u32, f32, f32); 7] = [
        (48, 1.0, 0.45),
        (64, 1.8, 0.40),
        (72, 2.6, 0.35),
        (96, 4.0, 0.30),
        (128, 7.0, 0.28),
        (160, 14.0, 0.25),
        (192, 30.0, 0.22),
    ];
    let mut meshes: Vec<Option<MeshletMesh>> = (0..recipes.len()).map(|_| None).collect();
    pool.scope(|s| {
        for (i, slot) in meshes.iter_mut().enumerate() {
            let (segments, radius, roughness) = recipes[i];
            s.spawn(move |_| {
                let mesh =
                    procedural::asteroid(Seed::new(700 + i as u64), segments, radius, roughness);
                *slot = Some(MeshletMesh::build(&mesh));
            });
        }
    });
    let mut builder = MeshletSceneBuilder::new();
    let mesh_ids: Vec<_> = meshes
        .iter()
        .map(|m| builder.add_mesh(m.as_ref().expect("mesh built")))
        .collect();
    for (i, mesh) in meshes.iter().enumerate() {
        if let Some(mesh) = mesh {
            tracing::info!(mesh = i, levels = ?mesh.clusters_per_level, dag_triangles = mesh.dag_triangle_count, "cluster DAG");
        }
    }
    let mesh_ms = start.elapsed().as_millis();

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
        let c = centre(t);
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
        builder.add_instance(
            mesh_ids[mesh],
            Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, position),
        );
        placed += 1;
    }
    let scene = builder.build(&ctx.device)?;
    tracing::info!(
        meshes = recipes.len(),
        mesh_build_ms = mesh_ms,
        placement_attempts = attempts,
        instances = scene.instance_count,
        meshlets = scene.instance_meshlets(),
        total_triangles = scene.total_triangles,
        total_ms = start.elapsed().as_millis(),
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
        ..AppConfig::default()
    };
    forge_app::run(config, move |ctx| Ballad::new(ctx, args))
}

//! `meshlets`: the culling test bench. A grid of procedural asteroids rendered GPU-driven,
//! with every culling stage switchable so its effect can be measured and frozen.
//!
//! Controls: WASD/QE move, Shift fast, right mouse look, F freeze culling, C cone culling,
//! V frustum culling, O occlusion culling, M meshlet colours, Tab wireframe, Esc quit.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_app::{AppConfig, Context, Demo, FlyCamera, FrameInfo, Input};
use forge_core::Seed;
use forge_geom::{MeshletMesh, procedural};
use forge_render::meshlet::DrawParams;
use forge_render::{
    CullCamera, CullFlags, FrameStats, MeshletRenderer, MeshletScene, MeshletSceneBuilder,
};
use glam::{Mat4, Quat, Vec3};
use winit::keyboard::KeyCode;

#[derive(Parser, Debug, Clone)]
#[command(about = "GPU-driven meshlet culling test bench")]
struct Args {
    /// Instances per side of the grid (total = side × side × 2 layers).
    #[arg(long, default_value_t = 24)]
    side: u32,
    /// Cube-sphere segments per face (triangles per instance = 12 × detail²).
    #[arg(long, default_value_t = 96)]
    detail: u32,
    /// Surface roughness (0 = smooth sphere; wide normal cones defeat cone culling).
    #[arg(long, default_value_t = 0.35)]
    roughness: f32,
    /// Vertical sync.
    #[arg(long)]
    vsync: bool,
    /// Vulkan validation layer.
    #[arg(long)]
    validate: bool,
    /// Start with occlusion culling disabled.
    #[arg(long)]
    no_occlusion: bool,
    /// Projected LOD error a drawn cluster may have, in pixels.
    #[arg(long, default_value_t = 1.0)]
    lod_error: f32,
    /// Draw the full-detail clusters only (no LOD selection; L toggles it).
    #[arg(long)]
    no_lod: bool,
    /// Scripted camera: turn and drift so every frame differs (for headless comparisons).
    #[arg(long)]
    orbit: bool,
    /// Exit after this many frames.
    #[arg(long)]
    frames: Option<u64>,
    /// Write a PNG of frame `--capture-frame` to this path.
    #[arg(long)]
    capture: Option<PathBuf>,
    /// Which frame to capture.
    #[arg(long, default_value_t = 60)]
    capture_frame: u64,
    /// Force the profiling overlay on (also in scripted runs). F1 toggles it.
    #[arg(long)]
    overlay: bool,
}

struct Bench {
    args: Args,
    renderer: MeshletRenderer,
    scene: MeshletScene,
    camera: FlyCamera,
    flags: CullFlags,
    frozen: Option<CullCamera>,
    wireframe: bool,
    stats: Vec<FrameStats>,
    gpu_ms: Vec<f64>,
    cpu_ms: Vec<f64>,
    title_updates: u32,
}

impl Bench {
    fn new(ctx: &mut Context, args: Args) -> Result<Self> {
        if !ctx.device.features().mesh_shader {
            anyhow::bail!(
                "{} has no mesh shader support (VK_EXT_mesh_shader)",
                ctx.device.name()
            );
        }
        let renderer = MeshletRenderer::new(
            &ctx.device,
            &ctx.shaders,
            ctx.swapchain.format(),
            ctx.extent(),
        )?;
        let scene = build_scene(ctx, &args)?;
        let side = args.side as f32;
        let camera = FlyCamera {
            position: Vec3::new(0.0, 12.0, side * 3.0 + 20.0),
            pitch: -0.25,
            ..FlyCamera::default()
        };
        let mut flags = CullFlags(CullFlags::CONE | CullFlags::FRUSTUM | CullFlags::MESHLET_COLORS);
        if !args.no_lod {
            flags.0 |= CullFlags::LOD;
        }
        if !args.no_occlusion {
            flags.0 |= CullFlags::OCCLUSION;
        }
        Ok(Self {
            args,
            renderer,
            scene,
            camera,
            flags,
            frozen: None,
            wireframe: false,
            stats: Vec::new(),
            gpu_ms: Vec::new(),
            cpu_ms: Vec::new(),
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

impl Demo for Bench {
    fn resized(&mut self, ctx: &mut Context) -> Result<()> {
        self.renderer.resize(ctx.extent())?;
        self.frozen = None;
        self.flags.0 &= !CullFlags::FREEZE;
        Ok(())
    }

    fn key_pressed(&mut self, ctx: &mut Context, code: KeyCode) {
        match code {
            KeyCode::KeyF => {
                self.flags.toggle(CullFlags::FREEZE);
                if self.flags.has(CullFlags::FREEZE) {
                    self.frozen = Some(self.cull_camera(ctx.aspect()));
                }
            }
            KeyCode::KeyC => self.flags.toggle(CullFlags::CONE),
            KeyCode::KeyV => self.flags.toggle(CullFlags::FRUSTUM),
            KeyCode::KeyO => self.flags.toggle(CullFlags::OCCLUSION),
            KeyCode::KeyM => self.flags.toggle(CullFlags::MESHLET_COLORS),
            KeyCode::KeyL => self.flags.toggle(CullFlags::LOD),
            KeyCode::KeyK => self.flags.toggle(CullFlags::LOD_COLORS),
            KeyCode::BracketLeft => self.args.lod_error = (self.args.lod_error * 0.5).max(0.125),
            KeyCode::BracketRight => self.args.lod_error = (self.args.lod_error * 2.0).min(16.0),
            KeyCode::Tab => self.wireframe = !self.wireframe,
            _ => {}
        }
    }

    fn update(&mut self, _ctx: &mut Context, input: &Input, dt: f32) {
        self.camera.update(input, dt);
        if self.args.orbit {
            // Deterministic per frame (not per second) so captures at a frame index match.
            self.camera.yaw += 0.004;
            self.camera.position += self.camera.forward() * 0.05;
        }
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
                "drawn: {} instances, {:.0} k + {:.0} k meshlets, {:.2} M triangles, {:.0} k occluded; LOD {} at {:.2} px, mean level {:.2}",
                last.instances_visible,
                f64::from(last.meshlets_pass1) / 1e3,
                f64::from(last.meshlets_pass2) / 1e3,
                f64::from(last.triangles) / 1e6,
                f64::from(last.occluded) / 1e3,
                if self.flags.has(CullFlags::LOD) { "on" } else { "off" },
                self.args.lod_error,
                f64::from(last.lod_level_sum) / f64::from((last.meshlets_pass1 + last.meshlets_pass2).max(1))
            ));
        }
        let live = self.cull_camera(ctx.aspect());
        let cull = match self.frozen {
            Some(frozen) if self.flags.has(CullFlags::FREEZE) => frozen,
            _ => live,
        };
        self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: live.view_proj,
                cull,
                lod_threshold_px: self.args.lod_error,
                draw_jitter: glam::Vec2::ZERO,
                flags: self.flags,
                color: frame.target,
                extent: ctx.extent(),
                clear_color: Some([0.02, 0.02, 0.03, 1.0]),
                wireframe: self.wireframe,
            },
        )?;
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
        let title = format!(
            "forge meshlets | {} inst × {} meshlets = {:.1} M meshlets, {:.0} M tris | drawn: {:.0} inst, {:.0} k + {:.0} k meshlets, {:.2} M tris, {:.0} k occluded | GPU {:.2} ms  CPU {:.2} ms | {}{}{}{}{}{}",
            self.scene.instance_count,
            self.scene.max_meshlets,
            self.scene.instance_meshlets() as f64 / 1e6,
            self.scene.total_triangles as f64 / 1e6,
            mean(|s| s.instances_visible),
            mean(|s| s.meshlets_pass1) / 1e3,
            mean(|s| s.meshlets_pass2) / 1e3,
            mean(|s| s.triangles) / 1e6,
            mean(|s| s.occluded) / 1e3,
            gpu,
            cpu,
            if self.flags.has(CullFlags::FREEZE) {
                "[F frozen] "
            } else {
                ""
            },
            if self.flags.has(CullFlags::CONE) {
                "[C cone] "
            } else {
                ""
            },
            if self.flags.has(CullFlags::FRUSTUM) {
                "[V frustum] "
            } else {
                ""
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

fn build_scene(ctx: &Context, args: &Args) -> Result<MeshletScene> {
    let start = Instant::now();
    let mesh = procedural::asteroid(Seed::new(2026), args.detail, 1.6, args.roughness);
    let built = MeshletMesh::build(&mesh);
    let cullable = built
        .meshlets
        .iter()
        .filter(|m| m.cone_cutoff < 1.0)
        .count();
    let mean_cutoff = built
        .meshlets
        .iter()
        .map(|m| f64::from(m.cone_cutoff))
        .sum::<f64>()
        / built.meshlets.len().max(1) as f64;
    tracing::info!(
        vertices = built.vertices.len(),
        triangles = built.triangle_count,
        meshlets = built.meshlets.len(),
        cone_cullable_pct = (cullable as f64 * 100.0 / built.meshlets.len().max(1) as f64).round(),
        mean_cone_cutoff = format!("{mean_cutoff:.3}"),
        ms = start.elapsed().as_millis(),
        "asteroid mesh built"
    );
    let mut builder = MeshletSceneBuilder::new();
    let mesh_id = builder.add_mesh(&built);
    let mut rng = Seed::new(99).rng();
    let side = args.side;
    let spacing = 7.0_f32;
    for layer in 0..2_u32 {
        for z in 0..side {
            for x in 0..side {
                let jitter = Vec3::new(
                    rng.range_f32(-1.5, 1.5),
                    rng.range_f32(-1.5, 1.5),
                    rng.range_f32(-1.5, 1.5),
                );
                let position = Vec3::new(
                    (x as f32 - side as f32 * 0.5) * spacing,
                    layer as f32 * 12.0 - 6.0,
                    (z as f32 - side as f32 * 0.5) * spacing,
                ) + jitter;
                let scale = rng.range_f32(0.6, 1.4);
                let rotation = Quat::from_euler(
                    glam::EulerRot::XYZ,
                    rng.range_f32(0.0, std::f32::consts::TAU),
                    rng.range_f32(0.0, std::f32::consts::TAU),
                    rng.range_f32(0.0, std::f32::consts::TAU),
                );
                builder.add_instance(
                    mesh_id,
                    Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, position),
                );
            }
        }
    }
    let scene = builder.build(&ctx.device)?;
    tracing::info!(
        instances = scene.instance_count,
        meshlets_per_instance = scene.max_meshlets,
        total_triangles = scene.total_triangles,
        "scene ready"
    );
    Ok(scene)
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config = AppConfig {
        title: "forge meshlets".into(),
        vsync: args.vsync,
        validate: args.validate,
        frame_limit: args.frames,
        capture: args.capture.clone().map(|p| (p, args.capture_frame)),
        capture_every: None,
        overlay: if args.overlay { Some(true) } else { None },
        ..AppConfig::default()
    };
    forge_app::run(config, move |ctx| Bench::new(ctx, args))
}

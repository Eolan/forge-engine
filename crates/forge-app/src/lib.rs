//! Application shell for demos and tools: a window, input state, the frame loop on
//! `forge_gpu::Frames`, swapchain recreation, PNG capture and a fly camera.
//!
//! A demo implements [`Demo`] and calls [`run`], or [`run_loading`] when its start-up has heavy
//! CPU work to do behind a loading screen. The shell owns the GPU context and the
//! render graph of every frame: it imports the swapchain image, lets the demo declare its
//! passes, adds the overlay, the capture and the present transition, and executes the
//! graph, which derives every barrier.

#![forbid(unsafe_code)]

pub mod alloc_count;
mod calibration;
mod camera;
pub mod console;
mod content_light;
mod hdr;
pub mod hud;
mod input;
mod loading;
mod menu;
mod overlay;
pub mod presets;
mod profile;
pub mod settings;
pub mod sound;
mod ssaa;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use calibration::{Action, Calibration, Page, SettingsFile};
pub use camera::{FlyCamera, SpringArm};
use content_light::ContentLightMeter;
/// Re-exported: what the OS says of the display, in [`DisplayOutput`].
pub use forge_gpu::DisplayCaps;
/// Re-exported: what an HDR swapchain tells the display ([`DisplayOutput::metadata`]).
pub use forge_gpu::HdrMetadata;
/// Re-exported for demos that declare their own per-frame targets.
pub use forge_gpu::TransientDesc;
/// Re-exported so demos can name Vulkan types without depending on `forge-gpu` directly.
pub use forge_gpu::vk;
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, Device, DeviceOptions, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, Frames, GraphBuffer, GraphStats, ImageAccess, ImageHandle, Instance, MemoryCategory,
    MemoryLocation, Pacing, RawImage, RenderGraph, ResourceState, ShaderCompiler, Surface,
    SurfaceMode, Swapchain, VENDOR_NVIDIA,
};
pub use hdr::{
    ContentLight, DisplayOutput, DisplaySettings, HdrMode, OFFSCREEN_FORMAT, PEAKS, preset_peak,
};
pub use input::Input;
pub use loading::Finish;
pub use overlay::{Canvas, Color, Overlay};
pub use profile::{MemorySample, OverlayMode, Profile};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
pub use settings::{Settings, SettingsArgs, Spec, Var};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// How to start the application.
#[derive(Clone, Debug)]
pub struct AppConfig {
    /// Window title (demos overwrite it with live statistics).
    pub title: String,
    /// Initial window size.
    pub width: u32,
    /// Initial window size.
    pub height: u32,
    /// Vertical sync. Without it, interactive runs present as fast as they draw without tearing
    /// and scripted runs (a frame limit) uncapped ([`Pacing`]).
    pub vsync: bool,
    /// Vulkan validation layer (always on in debug builds).
    pub validate: bool,
    /// Exit after this many frames.
    pub frame_limit: Option<u64>,
    /// Write this frame to this PNG.
    pub capture: Option<(PathBuf, u64)>,
    /// Also write every N-th frame to the capture path with `-NNNNN` appended to its stem
    /// (a sequence for finding the first frame where two runs diverge).
    pub capture_every: Option<u64>,
    /// Compile shaders with optimisation.
    pub optimize_shaders: bool,
    /// Whether the profiling overlay starts visible: `None` = no (the owner, 2026-10-19: the
    /// console's `r.display_info` or F1 shows it); `Some` forces it. F1 steps through it at run
    /// time.
    pub overlay: Option<bool>,
    /// Load the Vulkan API through NVIDIA Streamline so the device can offer DLSS (needs the
    /// `dlss` feature on Windows and the SDK in `streamline-sdk/bin/x64`, or wherever
    /// `FORGE_STREAMLINE_DIR` points). Falls back to the plain loader when Streamline does not
    /// load.
    pub streamline: bool,
    /// Create the device without `VK_EXT_mesh_shader` even when the GPU has it, so the
    /// renderers take their fallback paths (`--force-fallback`).
    pub force_fallback: bool,
    /// The HDR output asked for (issue #94; `FORGE_HDR=off|hdr10|scrgb|offscreen` overrides
    /// it). On the display it needs the OS to show the display in HDR; `app.hdr` switches at run
    /// time.
    pub hdr: HdrMode,
    /// The paper-white offset: stops added to the scene before ACES 2.0's HDR presets (0, the
    /// Academy's look, D-022; `--hdr-stops`).
    pub hdr_stops: f32,
    /// The UI's white in HDR, in nits, over the calibration's and the OS's (`--hdr-ui-white`).
    pub hdr_ui_white: Option<f32>,
    /// Supersample 2 × 2 (D-045, for screenshots): the demo draws at twice the window's width
    /// and height ([`Context::extent`]) and the shell filters the frame down. Not with the
    /// off-screen HDR mode.
    pub ssaa: bool,
    /// The settings given at start (D-069): `--set NAME=VALUE` and the settings file, applied
    /// to [`Context::settings`] as each variable registers.
    pub settings: SettingsArgs,
    /// The demo plays sound (#318): an interactive run opens the default output device
    /// ([`Context::sound`]). Scripted runs (a frame limit, a capture) and `FORGE_SOUND=0` never
    /// do.
    pub sound: bool,
    /// Writes the run's mix to this WAV file instead of playing it (`--sound-record`): scripted
    /// runs too, never a device. For checks of what a scene sounds like.
    pub sound_record: Option<PathBuf>,
    /// Seconds of sound each frame of a recorded run renders: the demo's fixed step
    /// (`--fixed-step`), so the sound keeps time with the scene. None: the frame's own time.
    pub sound_record_step: Option<f32>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "forge".to_owned(),
            width: 1600,
            height: 900,
            vsync: false,
            validate: false,
            frame_limit: None,
            capture: None,
            capture_every: None,
            optimize_shaders: true,
            overlay: None,
            streamline: false,
            force_fallback: false,
            hdr: HdrMode::Off,
            hdr_stops: 0.0,
            hdr_ui_white: None,
            ssaa: false,
            settings: SettingsArgs::default(),
            sound: false,
            sound_record: None,
            sound_record_step: None,
        }
    }
}

/// Everything a demo needs from the shell.
pub struct Context {
    /// The device.
    pub device: Arc<Device>,
    /// The swapchain (recreated on resize; watch [`Context::extent`]).
    pub swapchain: Swapchain,
    /// The frame target's format and the HDR settings (issue #94): display passes follow it
    /// every frame (forge-render's `set_output`).
    pub output: DisplayOutput,
    /// Frames in flight.
    pub frames: Frames,
    /// The shader compiler rooted at the workspace `shaders/` directory.
    pub shaders: ShaderCompiler,
    /// The window.
    pub window: Arc<Window>,
    /// Frames rendered so far.
    pub frames_rendered: u64,
    /// Frames saved as captures so far (`--capture`, `--capture-every`, a demo's request): a
    /// capture's wait and PNG slow the frame it ends, so a demo's frame-time statistics leave
    /// out the frames during which this count grew.
    pub captures_written: u64,
    /// The frame profile behind the overlay; demos add counter lines to it.
    pub profile: Profile,
    /// The render graph's persistent side (transient heap, statistics).
    pub graph: RenderGraph,
    /// A capture the demo asks for: the next frame is saved to this path as `--capture` saves
    /// its frame (and its HDR codes beside it when offscreen), then the request clears. The
    /// planet's swap check saves the frames either side of a swap this way (#220).
    pub capture_request: Option<PathBuf>,
    /// While a demo started with [`run_loading`] prepares on its thread (issue #25): the loading
    /// screen shows, and its frames do not count (`frames_rendered`, captures, the frame limit,
    /// the profile, the memory counters).
    pub loading: bool,
    /// The demo's own preparation on the GPU once started (an impostor's bake, D-075 step 4):
    /// while the demo sets it, its frames do not count (`frames_rendered`, captures, the frame
    /// limit, the profile), so a scripted run waits for it; they are drawn as usual, under the
    /// loading screen in an interactive run started with [`run_loading`].
    pub preparing: bool,
    /// The console variables and commands (D-069): the shell's (`r.display_info`, `app.`), the
    /// renderer's and the demo's, registered in [`Demo::register`] and read every frame.
    pub settings: Settings,
    /// The sound output, when the demo asked for it and the run is interactive (#318).
    pub sound: Option<sound::Sound>,
    /// The game's HUD this frame (#328): drawn after the frame, under the debug overlay, in
    /// order, then emptied ([`hud`]).
    pub hud: Vec<hud::HudQuad>,
    /// A lens over the picture this frame (#328, a scope's), under the HUD's quads; emptied
    /// after it is drawn.
    pub magnifier: Option<hud::HudMagnifier>,
    /// The game's menu is open over a solo game (#352): the game holds behind it. The shell
    /// passes the update a zero `dt`; a demo that steps at a fixed rate of its own skips its
    /// steps while this is set.
    pub paused: bool,
    /// The HUD's glyphs (the menu's, [`Context::hud_text`]'s), made the first time they are
    /// needed: `Some(None)` when they could not be.
    glyphs: Option<Option<menu::MenuFont>>,
    /// The font they are made from, the overlay's.
    font_path: PathBuf,
    /// The frame's size over the window's, each way: 1, or [`ssaa::FACTOR`] when supersampling.
    ssaa: u32,
    /// The first frame (in the frame slots' numbering) the demo recorded after the loading
    /// screen: the GPU zones of earlier frames are the loading screen's, not the demo's.
    counted_from: u64,
    _surface: Arc<Surface>,
    _instance: Arc<Instance>,
}

impl Context {
    /// The size the demo draws its frame at: the swapchain's, or twice it each way when
    /// supersampling ([`AppConfig::ssaa`]).
    pub fn extent(&self) -> vk::Extent2D {
        let e = self.swapchain.extent();
        vk::Extent2D {
            width: e.width * self.ssaa,
            height: e.height * self.ssaa,
        }
    }

    /// Aspect ratio of the swapchain.
    pub fn aspect(&self) -> f32 {
        let e = self.extent();
        e.width as f32 / e.height.max(1) as f32
    }

    /// What a demo's finishing step reads of the shell ([`Setup`]), as it stands now.
    pub fn setup(&self) -> Setup {
        Setup {
            device: Arc::clone(&self.device),
            shaders: self.shaders.clone(),
            output: self.output,
            extent: self.extent(),
        }
    }

    /// The HUD's size: the window's pixels, which [`hud::HudQuad`]'s rectangles are in.
    pub fn hud_size(&self) -> [f32; 2] {
        let e = self.swapchain.extent();
        [e.width as f32, e.height as f32]
    }

    /// `text` on the HUD this frame, in the glyphs of the game's menu (#352, the overlay's
    /// font): its first cell's top left at `at` (window pixels), `px` pixels high, in `color`
    /// (linear, straight alpha). Printable ASCII; other characters show as `?`. Its width in
    /// pixels (0 when the glyphs could not be made).
    pub fn hud_text(&mut self, at: [f32; 2], px: f32, color: [f32; 4], text: &str) -> f32 {
        self.make_glyphs();
        let Some(Some(font)) = &self.glyphs else {
            return 0.0;
        };
        font.text(&mut self.hud, at, px, color, text);
        font.metrics().width(px, text)
    }

    /// How wide [`Context::hud_text`] draws `text` at `px` pixels.
    pub fn hud_text_width(&mut self, px: f32, text: &str) -> f32 {
        self.make_glyphs();
        match &self.glyphs {
            Some(Some(font)) => font.metrics().width(px, text),
            _ => 0.0,
        }
    }

    /// The HUD's glyphs, made from the overlay's font the first time they are needed.
    fn make_glyphs(&mut self) {
        if self.glyphs.is_none() {
            self.glyphs = Some(
                menu::MenuFont::new(&self.device, Some(&self.font_path))
                    .inspect_err(|e| tracing::warn!("the HUD's glyphs: {e}"))
                    .ok(),
            );
        }
    }

    /// The glyphs' metrics, once made.
    fn glyph_metrics(&self) -> Option<menu::Metrics> {
        self.glyphs.as_ref()?.as_ref().map(|f| f.metrics())
    }
}

/// What a demo's finishing step has of the shell (#201): the device, the shader compiler, the
/// frame target's format and HDR settings, and the frame's size when it started. It runs on a
/// worker while the loading screen keeps drawing on the main thread, so it gets these rather
/// than the [`Context`] (the swapchain, the frames and the graph stay the loading screen's).
#[derive(Clone)]
pub struct Setup {
    /// The device.
    pub device: Arc<Device>,
    /// The shader compiler rooted at the workspace `shaders/` directory.
    pub shaders: ShaderCompiler,
    /// The frame target's format and the HDR settings.
    pub output: DisplayOutput,
    extent: vk::Extent2D,
}

impl Setup {
    /// The size the demo draws its frame at, as [`Context::extent`] gave it.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    /// Its aspect ratio.
    pub fn aspect(&self) -> f32 {
        let e = self.extent;
        e.width as f32 / e.height.max(1) as f32
    }
}

/// One frame handed to [`Demo::render`].
pub struct FrameInfo<'f> {
    /// The frame's render graph: the demo declares its passes into it.
    pub graph: FrameGraph<'f>,
    /// The image to draw the frame into (contents undefined on entry), of
    /// [`Context::output`]'s format: the swapchain image, or in the off-screen HDR mode an
    /// HDR10 image the shell previews on it. The demo's last pass on it must write it; the
    /// shell adds the overlay, the capture and the present transition after.
    pub target: ImageHandle,
    /// The frame slot (index, number, previous GPU time).
    pub slot: FrameSlot,
    /// Index of the swapchain image being rendered.
    pub image_index: u32,
    /// Seconds since the previous frame (clamped to 0.1).
    pub dt: f32,
}

/// A demo or tool driven by the shell. Construction happens in the closure given to
/// [`run`], once the window and device exist.
pub trait Demo: Sized + 'static {
    /// The swapchain changed size: recreate size-dependent resources. The device is idle.
    fn resized(&mut self, _ctx: &mut Context) -> Result<()> {
        Ok(())
    }

    /// A key went down (not repeated). Escape is handled by the shell.
    fn key_pressed(&mut self, _ctx: &mut Context, _code: KeyCode) {}

    /// Per-frame simulation with the current input.
    fn update(&mut self, _ctx: &mut Context, _input: &Input, _dt: f32) {}

    /// Declares the frame's passes into `frame.graph`. The pass bodies borrow the demo for
    /// the frame (`'f`), so per-frame mutable state is updated here, before the passes run.
    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()>;

    /// Statistics line for the window title, polled four times per second.
    fn title(&mut self, _ctx: &Context) -> Option<String> {
        None
    }

    /// Registers the demo's console variables and commands into [`Context::settings`]
    /// (D-069), once, on the main thread before its first frame (after the loading screen for
    /// [`run_loading`]).
    fn register(&mut self, _ctx: &mut Context) {}

    /// Whether the game runs with others (a networked session): its menu (#352) then leaves it
    /// running; a solo game holds behind the menu ([`Context::paused`]).
    fn networked(&self) -> bool {
        false
    }

    /// Runs a console command the demo registered: what to print, or why it failed. `None`
    /// when the command is not the demo's.
    fn command(
        &mut self,
        _ctx: &mut Context,
        _name: &str,
        _args: &[String],
    ) -> Option<std::result::Result<String, String>> {
        None
    }
}

/// Runs a demo whose start-up has heavy CPU work (issue #25): `prepare` runs on a thread of its
/// own while the window shows a loading animation, and returns the step that finishes the demo
/// (uploads, pipelines), which runs on a worker of its own too while the animation goes on
/// (#201), with the [`Setup`] of the shell. The loading frames do not count: the demo's frames
/// number from 0, as with [`run`], so frame limits and captures are unchanged.
pub fn run_loading<D: Demo + Send + 'static>(
    config: AppConfig,
    prepare: impl FnOnce() -> Result<Finish<D>> + Send + 'static,
) -> Result<()> {
    init_tracing();
    // The preparation starts at once, beside the window's and the device's creation, which it
    // does not need: Streamline's start alone took 4.1 s of the village's 7.2 (#274). Its steps
    // are recorded from here, kept for the load profile when `dev.load_profile` asks for one
    // (#345) and dropped at the shell's start otherwise.
    forge_core::activity::record_start();
    let preparing = loading::prepare(prepare)?;
    // The shaders compiled ahead start there too (slangc needs no device either: a fresh cache
    // compiles for seconds after a shader's edit), and the shell takes the same compiler.
    let shaders = shader_compiler(&config)?;
    let warm_up = loading::warm_up(shaders.clone())?;
    // Interactive runs draw the demo's first frames behind the loading screen (#342); scripted
    // ones show them, their frames counted from the first.
    let scripted =
        config.frame_limit.is_some() || config.capture.is_some() || config.capture_every.is_some();
    let settle = if scripted { 0 } else { loading::SETTLE_FRAMES };
    run_with(config, Some(shaders), move |ctx| {
        loading::Stage::start(ctx, preparing, warm_up, settle)
    })
}

/// The shell's shader compiler: the workspace's `shaders/`, the derived cache's.
fn shader_compiler(config: &AppConfig) -> Result<ShaderCompiler> {
    let root = workspace_root_from(env!("CARGO_MANIFEST_DIR"));
    Ok(ShaderCompiler::new(
        root.join("shaders"),
        forge_core::derived::cache_dir(forge_core::derived::CacheKind::Shaders),
        config.optimize_shaders,
    )?)
}

/// The log, to the console, filtered by `RUST_LOG` ("info" by default); once.
fn init_tracing() {
    if !tracing::dispatcher::has_been_set() {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .init();
    }
}

/// Runs the demo built by `init` until the window closes, the game's menu quits (#352) or the
/// frame limit is reached. The demo and the shell's resources are dropped before it returns, but the
/// GPU device itself is left for the process's end to reclaim (#341), unless validating or with
/// `FORGE_TEARDOWN=1`: a host that runs it more than once in a process sets that.
pub fn run<D: Demo>(
    config: AppConfig,
    init: impl FnOnce(&mut Context) -> Result<D> + 'static,
) -> Result<()> {
    run_with(config, None, init)
}

/// [`run`], the shell taking `shaders` when given ([`run_loading`] makes it at the start).
fn run_with<D: Demo>(
    config: AppConfig,
    shaders: Option<ShaderCompiler>,
    init: impl FnOnce(&mut Context) -> Result<D> + 'static,
) -> Result<()> {
    init_tracing();
    // The frames first: work beside them (a planet's tiles cooked as the camera flies, the
    // city's streaming) takes the cores they leave. The cooking threads below normal measured
    // worse, the frame thread above normal half the slow frames (#220, `forge_task::priority`).
    forge_task::raise_current_thread_priority();
    #[cfg(feature = "profiling")]
    let _tracy = {
        let client = tracy_client::Client::start();
        tracy_client::set_thread_name!("main");
        tracing::info!("Tracy client started: connect tracy-profiler to see zones");
        client
    };
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::<D> {
        config,
        init: Some(Box::new(init)),
        shaders,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    if let Some(mut state) = app.state.take() {
        let frames = state.ctx.frames_rendered;
        state.sample_memory();
        if let Some(memory) = state.ctx.profile.memory() {
            tracing::info!("memory: {}", memory.summary());
        }
        if let Some(gpu) = state.ctx.profile.gpu_run_summary() {
            tracing::info!("gpu: {gpu}");
        }
        if let Some(cpu) = state.ctx.profile.cpu_run_summary() {
            tracing::info!("cpu: {cpu}");
        }
        if let Some(mean) = state.allocs.mean() {
            tracing::info!(
                "allocations: {mean:.1} per frame over {} frames",
                state.allocs.frames()
            );
        }
        if let Some(light) = state.ctx.output.content_light {
            tracing::info!(
                "hdr: MaxCLL {:.1} nits, MaxFALL {:.1} nits",
                light.max_cll,
                light.max_fall
            );
        }
        // The device's own teardown (`vkDestroyDevice`, the allocator's blocks, Streamline) is
        // left to the process's end, which reclaims it at once: after a long run the driver took
        // 0.4–1.9 s over it, growing with the frames run (#341), and a game's exe stayed locked
        // meanwhile. Everything else drops as before, the demo's own state first. Validation
        // runs (they report what was left undestroyed) and `FORGE_TEARDOWN=1` destroy it too.
        if !state.config.validate && std::env::var_os("FORGE_TEARDOWN").is_none() {
            std::mem::forget(Arc::clone(&state.ctx.device));
        }
        drop(state);
        tracing::info!(frames, "exited cleanly");
    }
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn ms_since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e3
}

/// The workspace root (two levels above a crate manifest in `crates/` or `demos/`).
pub fn workspace_root_from(manifest_dir: &str) -> PathBuf {
    let manifest = PathBuf::from(manifest_dir);
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where the app keeps its settings files (`display.txt`): `FORGE_SETTINGS_DIR` when it is set,
/// for a tool outside the repository that builds on the engine's crates (#289), else
/// `settings/` in the workspace at `root`.
pub fn settings_dir(root: &std::path::Path) -> PathBuf {
    std::env::var_os("FORGE_SETTINGS_DIR")
        .filter(|v| !v.is_empty())
        .map_or_else(|| root.join("settings"), PathBuf::from)
}

/// The OS's name of the window's monitor (Windows' `\\.\DISPLAYn`), for [`DisplayCaps`].
fn monitor_name(window: &Window) -> Option<String> {
    #[cfg(windows)]
    {
        use winit::platform::windows::MonitorHandleExtWindows;
        window.current_monitor().map(|monitor| monitor.native_id())
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}

/// The shell's HDR passes besides the demo's (issues #94, #125).
struct HdrPasses {
    /// The off-screen mode's preview, made when the mode is first used.
    preview: Option<hdr::Preview>,
    /// MaxCLL and MaxFALL from the frames shown, made when HDR is first used.
    meter: Option<ContentLightMeter>,
    /// The calibration pages (`app.calibrate`).
    calibration: Calibration,
    /// Supersampling's last pass ([`AppConfig::ssaa`]), on the window's format.
    ssaa: Option<ssaa::Ssaa>,
}

/// Switches the output to `mode` (issue #94). HDR10 and scRGB recreate the swapchain in that
/// mode when the OS shows the display in HDR (and stay off otherwise); off-screen keeps the
/// swapchain SDR and has the frame drawn into an HDR10 image. Waits for the device whenever the
/// target's format changes, so the display passes can rebuild their pipelines. The content light
/// starts again from the new mode's frames (#125). Returns whether the swapchain was recreated.
fn apply_hdr_mode(
    ctx: &mut Context,
    overlay: &mut Overlay,
    passes: &mut HdrPasses,
    mut mode: HdrMode,
) -> Result<bool> {
    let on_display = matches!(mode, HdrMode::Hdr10 | HdrMode::ScRgb);
    if on_display && ctx.output.caps.is_some_and(|c| !c.hdr_on) {
        tracing::warn!(
            asked = mode.name(),
            "the OS shows this display in SDR (Windows: turn on \"Use HDR\"): HDR stays off"
        );
        mode = HdrMode::Off;
    }
    let wanted = match mode {
        HdrMode::Hdr10 => SurfaceMode::Hdr10,
        HdrMode::ScRgb => SurfaceMode::ScRgb,
        HdrMode::Off | HdrMode::Offscreen => SurfaceMode::Sdr,
    };
    let recreated = wanted != ctx.swapchain.mode();
    if recreated {
        if ctx.swapchain.set_mode(wanted)? != wanted {
            mode = HdrMode::Off;
        }
        ctx.frames.resize_swapchain(ctx.swapchain.image_count())?;
    }
    let format = if mode == HdrMode::Offscreen {
        OFFSCREEN_FORMAT
    } else {
        ctx.swapchain.format()
    };
    if format != ctx.output.format {
        ctx.device.wait_idle();
    }
    ctx.output.mode = mode;
    ctx.output.format = format;
    ctx.output.content_light = None;
    if mode == HdrMode::Offscreen && passes.preview.is_none() {
        passes.preview = Some(hdr::Preview::new(
            &ctx.device,
            &ctx.shaders,
            ctx.swapchain.format(),
        )?);
    }
    if mode != HdrMode::Off && passes.meter.is_none() {
        passes.meter = Some(ContentLightMeter::new(&ctx.device, &ctx.shaders)?);
    }
    if let Some(meter) = &mut passes.meter {
        meter.forget();
    }
    if passes.calibration.page().is_some() {
        if mode == HdrMode::Off {
            passes.calibration.close();
        } else {
            passes
                .calibration
                .prepare(&ctx.device, &ctx.shaders, format)?;
        }
    }
    ctx.swapchain.set_hdr_metadata(ctx.output.metadata());
    overlay.set_output(&ctx.shaders, ctx.swapchain.format(), ctx.output.ui_white)?;
    tracing::info!("{}", ctx.output.describe());
    Ok(recreated)
}

/// The window as it was before it went fullscreen, to go back to.
#[derive(Clone, Copy, Debug)]
struct Windowed {
    /// Its outer corner on the desktop, where the platform says.
    position: Option<winit::dpi::PhysicalPosition<i32>>,
    /// Its inner size.
    size: winit::dpi::PhysicalSize<u32>,
    /// Whether it was maximised.
    maximized: bool,
}

struct State<D: Demo> {
    demo: D,
    ctx: Context,
    input: Input,
    needs_resize: Option<PhysicalSize<u32>>,
    /// The last acquire found the swapchain out of date: the next frame recreates it even at the
    /// window's size. A window as big as its monitor, which Windows moved off the monitor's
    /// bounds, kept its size while every acquire failed: no frame, a core spinning (the timings'
    /// 1440p asteroids, 2026-10-17).
    stale_swapchain: bool,
    last_frame: Instant,
    last_title: Instant,
    config: AppConfig,
    /// `FORGE_WAIT_IDLE=1`: wait for the device after every submit (debugging).
    debug_wait_idle: bool,
    /// Whether the window is shown: it is created hidden and shown after its first frame is
    /// presented, so it never shows the system's white background (#219).
    shown: bool,
    /// `FORGE_NO_TITLE=1`: never update the window title (debugging).
    debug_no_title: bool,
    /// `FORGE_STALL_MS=N`: sleep N ms after every frame (debugging).
    debug_stall_ms: u64,
    /// `FORGE_HDR_CYCLE=N`: `app.hdr` every N frames (the HDR switch in scripted runs).
    debug_hdr_cycle: u64,
    /// The frame of the last scripted switch (a frame can start twice, after a resize).
    last_hdr_cycle: u64,
    /// `FORGE_LOADING_CAPTURE=path`: one loading frame captured there, `FORGE_LOADING_CAPTURE_AT`
    /// seconds (2 by default) after the shell started, to check what the loading screen says;
    /// taken once, by a frame the demo prepares in ([`Context::preparing`]) if the loading is
    /// over by then.
    loading_capture: Option<(PathBuf, Instant)>,
    overlay: Overlay,
    /// The preview, the content-light meter and the calibration pages.
    hdr: HdrPasses,
    /// The HDR mode asked for at start (`--hdr`, `FORGE_HDR`): the one `app.hdr` turns on.
    hdr_requested: HdrMode,
    /// Where the calibration is saved, in interactive runs (#125), and the monitor it is the
    /// calibration of: the one the window opened on.
    settings_file: Option<SettingsFile>,
    monitor: String,
    /// The render graph's counters of the previous frame (shown in the overlay).
    graph_stats: GraphStats,
    /// When the memory counters were last sampled, and the traffic totals then.
    memory_mark: Option<MemoryMark>,
    /// The first sample (the first frame, after the demo's start-up uploads): the exit log
    /// averages the traffic over the run from it.
    memory_start: Option<MemoryMark>,
    /// The developer console (D-069) and the modifier keys it reads (Ctrl).
    console: console::Console,
    modifiers: winit::keyboard::ModifiersState,
    /// The window as it was before Alt+Enter made it fullscreen: where it was, its size, and
    /// whether it was maximised; None while windowed.
    windowed: Option<Windowed>,
    /// `r.display_info`: what the profiling overlay shows (0 off, 1 compact, 2 full; F1).
    display_info: Var<i64>,
    /// `app.console`: whether the console shows (the key under Esc).
    console_shown: Var<bool>,
    /// `app.mouse_look`: the game's mouse look (the cursor captured, the right button free).
    mouse_look: Var<bool>,
    /// The game's HUD (#328) and `app.hud`, whether it shows.
    hud: hud::Hud,
    hud_shown: Var<bool>,
    /// `snd.volume`: the master's level (#318), and the level last sent to the mixer: faded to
    /// silence behind the game's menu while the game holds (#352).
    volume: Var<f64>,
    volume_sent: f64,
    /// `snd.ambience`: the ambience's level under the master (#357), and the level last sent.
    ambience: Var<f64>,
    ambience_sent: f64,
    /// `r.preset` (the quality preset, [`presets`]) and the settings' change count when its
    /// patch was last applied: a patched setting changed by hand since puts it back to custom.
    preset: Var<usize>,
    preset_applied: u64,
    /// `app.window`: windowed or borderless fullscreen, followed each frame.
    window_mode: Var<usize>,
    /// The user's settings file (`settings/user.cfg`): the menu's choices and the first
    /// launch's, written again whenever one changes; `None` in a scripted run. Its leading
    /// comment lines are kept.
    user_file: Option<PathBuf>,
    user_header: String,
    /// The game's menu on Esc (#352), and where the cursor stands in the window, pixels.
    menu: menu::Menu,
    cursor: [f32; 2],
    /// The heap allocations per frame (`alloc-count`), logged at exit.
    allocs: alloc_count::PerFrame,
    /// The playing audio device, for as long as the run lasts.
    _sound_output: Option<sound::Output>,
    /// The mix written to a file instead (`AppConfig::sound_record`).
    sound_recorder: Option<sound::Recorder>,
    /// The overrides naming no variable were reported (once, after the demo registered its).
    unmatched_reported: bool,
    #[cfg(feature = "profiling")]
    tracy_gpu: Option<tracy_client::GpuContext>,
}

/// Traffic totals at one moment, to turn the device's running totals into rates.
#[derive(Clone, Copy)]
struct MemoryMark {
    at: Instant,
    frame: u64,
    uploaded: u64,
    read_back: u64,
}

/// How often the overlay's memory counters are refreshed: the budget query goes through the
/// driver to the OS.
const MEMORY_SAMPLE_PERIOD: Duration = Duration::from_millis(250);

type InitFn<D> = Box<dyn FnOnce(&mut Context) -> Result<D>>;

impl<D: Demo> State<D> {
    /// Refreshes the memory counters: the device's report, and the traffic per frame and per
    /// second since `since` (the previous sample by default).
    fn sample_memory_since(&mut self, since: Option<MemoryMark>) {
        let report = self.ctx.device.memory_report();
        let mark = MemoryMark {
            at: Instant::now(),
            frame: self.ctx.frames_rendered,
            uploaded: report.uploaded,
            read_back: report.read_back,
        };
        let (uploaded_per_frame, uploaded_per_second, read_back_per_frame) = match since {
            Some(since) => {
                let frames = mark.frame.saturating_sub(since.frame).max(1) as f64;
                let seconds = (mark.at - since.at).as_secs_f64().max(1e-6);
                let uploaded = mark.uploaded.saturating_sub(since.uploaded) as f64;
                let read_back = mark.read_back.saturating_sub(since.read_back) as f64;
                (uploaded / frames, uploaded / seconds, read_back / frames)
            }
            None => (0.0, 0.0, 0.0),
        };
        #[cfg(feature = "profiling")]
        {
            if let Some((usage, _)) = report.device_local() {
                tracy_client::plot!("VRAM MiB", usage as f64 / f64::from(1 << 20));
            }
            tracy_client::plot!("upload KiB per frame", uploaded_per_frame / 1024.0);
        }
        self.ctx.profile.set_memory(MemorySample {
            report,
            uploaded_per_frame,
            uploaded_per_second,
            read_back_per_frame,
        });
        self.memory_start.get_or_insert(mark);
        self.memory_mark = Some(mark);
    }

    /// Samples the memory with the traffic averaged over the whole run (the exit log).
    fn sample_memory(&mut self) {
        self.sample_memory_since(self.memory_start);
    }

    /// `app.hdr`: HDR on or off. On, it is the mode asked for at start, else HDR10 when the OS shows
    /// the display in HDR, else off-screen; the display is read again first (the window may
    /// have moved, or HDR been turned on).
    fn toggle_hdr(&mut self) -> Result<()> {
        let caps = monitor_name(&self.ctx.window).and_then(|name| DisplayCaps::query(&name));
        if caps != self.ctx.output.caps {
            tracing::info!(?caps, "display");
            self.ctx.output.refresh(caps);
        }
        let mode = match self.hdr_requested {
            _ if self.ctx.output.is_hdr() => HdrMode::Off,
            HdrMode::Off if caps.is_some_and(|c| c.hdr_on) => HdrMode::Hdr10,
            HdrMode::Off => HdrMode::Offscreen,
            asked => asked,
        };
        if apply_hdr_mode(&mut self.ctx, &mut self.overlay, &mut self.hdr, mode)? {
            self.demo.resized(&mut self.ctx)?;
        }
        Ok(())
    }

    /// `app.hdr.peak`: the next of ACES 2.0's peaks.
    fn next_peak(&mut self) {
        let peak = self.ctx.output.peak;
        self.ctx.output.peak = hdr::next_peak(peak);
        self.content_light_changed(peak);
        tracing::info!("{}", self.ctx.output.describe());
    }

    /// The preset's peak was `peak` and may have changed: the content light starts again (the
    /// curve's range is another), and the display is told.
    fn content_light_changed(&mut self, peak: f32) {
        if self.ctx.output.peak != peak {
            self.ctx.output.content_light = None;
            if let Some(meter) = &mut self.hdr.meter {
                meter.forget();
            }
        }
        self.ctx
            .swapchain
            .set_hdr_metadata(self.ctx.output.metadata());
    }

    /// `app.calibrate`: the calibration pages, from the first, in an HDR mode (#125).
    fn open_calibration(&mut self, page: Page) -> Result<()> {
        if !self.ctx.output.is_hdr() {
            tracing::warn!("the HDR calibration needs an HDR mode: app.hdr first");
            return Ok(());
        }
        self.hdr
            .calibration
            .open(&self.ctx.device, &self.ctx.shaders, &self.ctx.output, page)?;
        Ok(())
    }

    /// A key on the open calibration pages (`fine`: Shift is down).
    /// Opens the game's menu (#352), its glyphs made the first time: the keys and buttons held
    /// let go, the cursor the system's.
    fn open_menu(&mut self) {
        self.ctx.make_glyphs();
        self.menu.open();
        self.input.release_all();
        self.input.paused = true;
        self.input.looking = false;
        self.input.mouse_delta = (0.0, 0.0);
        free_cursor(&self.ctx.window);
    }

    /// Closes the menu: the game's mouse look takes the cursor again.
    fn close_menu(&mut self) {
        self.menu.close();
        self.input.paused = false;
        if self.input.mouse_look && !self.input.unfocused {
            self.input.looking = true;
            capture_cursor(&self.ctx.window);
        }
    }

    /// Does what the menu asked: whether to quit.
    fn menu_action(&mut self, action: Option<menu::Action>) -> bool {
        match action {
            Some(menu::Action::Resume) => {
                self.close_menu();
                false
            }
            Some(menu::Action::Quit) => true,
            None => false,
        }
    }

    /// Borderless fullscreen on the window's monitor, or back to the window as it was before:
    /// its place, its size, maximised or not (Alt+Enter and `app.fullscreen`, the owner).
    fn toggle_fullscreen(&mut self) {
        let window = &self.ctx.window;
        if window.fullscreen().is_none() {
            self.windowed = Some(Windowed {
                position: window.outer_position().ok(),
                size: window.inner_size(),
                maximized: window.is_maximized(),
            });
            window.set_fullscreen(Some(winit::window::Fullscreen::Borderless(
                window.current_monitor(),
            )));
            tracing::info!("fullscreen (borderless)");
            return;
        }
        window.set_fullscreen(None);
        if let Some(w) = self.windowed.take() {
            if w.maximized {
                window.set_maximized(true);
            } else {
                let _ = window.request_inner_size(w.size);
                if let Some(at) = w.position {
                    window.set_outer_position(at);
                }
            }
        }
        tracing::info!("windowed again");
    }

    /// `app.window` followed: the window made fullscreen or windowed as the setting says (the
    /// user's file at start, the menu's graphics page), and the setting following Alt+Enter.
    fn follow_window_mode(&mut self) {
        let wanted = self.ctx.settings.get(self.window_mode) == 1;
        let is = self.ctx.window.fullscreen().is_some();
        if wanted != is {
            // Alt+Enter changed the window, not the setting: the setting follows, for the run.
            if self.ctx.settings.source(self.window_mode) == settings::Source::Code
                || !self
                    .ctx
                    .settings
                    .changed_since(self.window_mode, self.preset_applied)
            {
                self.toggle_fullscreen();
            }
            let now = self.ctx.window.fullscreen().is_some();
            self.ctx.settings.set(self.window_mode, usize::from(now));
        }
    }

    /// `r.preset` followed ([`presets`]): a preset chosen sets its patch, as from where the
    /// preset came (the user's file, the menu, the console); a patched setting changed by hand
    /// since puts the preset back to custom.
    fn apply_preset(&mut self) {
        let s = &mut self.ctx.settings;
        let preset = s.get(self.preset);
        if s.changed_since(self.preset, self.preset_applied) {
            if preset != presets::CUSTOM {
                let source = s.source(self.preset);
                for (name, value) in presets::patch(presets::PRESETS[preset]) {
                    if let Err(e) = s.set_text(name, value, source) {
                        tracing::warn!("r.preset {}: {name}: {e}", presets::PRESETS[preset]);
                    }
                }
                tracing::info!(preset = presets::PRESETS[preset], "quality preset applied");
            }
            self.preset_applied = s.changes();
        } else if preset != presets::CUSTOM
            && s.vars()
                .any(|v| presets::is_patched(&v.name) && v.changed > self.preset_applied)
        {
            let source = s.source(self.preset);
            s.set_by(self.preset, presets::CUSTOM, source);
            self.preset_applied = s.changes();
        }
    }

    /// Writes the user's file from the settings that are the user's ([`settings::Source::User`]),
    /// under its comment lines.
    fn write_user_file(&self) {
        let Some(path) = &self.user_file else {
            return;
        };
        let mut text = self.user_header.clone();
        for line in self.ctx.settings.user_lines() {
            text.push_str(&line);
            text.push('\n');
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(path, text) {
            Ok(()) => tracing::info!(path = %path.display(), "user settings written"),
            Err(e) => tracing::warn!(path = %path.display(), "user settings not written: {e}"),
        }
    }

    /// Runs a console command (D-069): the shell's own (`app.`), else the demo's. Returns
    /// whether the window should close.
    fn run_command(&mut self, name: &str, args: &[String]) -> Result<bool> {
        match name {
            "app.quit" => return Ok(true),
            "app.menu" => self.open_menu(),
            "app.fullscreen" => self.toggle_fullscreen(),
            "app.capture" => {
                let path = PathBuf::from(args.first().map_or("capture.png", String::as_str));
                self.console.print(
                    format!("saving the next frame to {}", path.display()),
                    Color::Green,
                );
                self.ctx.capture_request = Some(path);
            }
            // The HDR output (issue #94): on and off, the peak, the preview's false colours, the
            // calibration pages (#125). Keys F2 to F5 until D-069.
            "app.hdr" => {
                self.toggle_hdr()?;
                let text = self.ctx.output.describe();
                self.console.print(text, Color::Green);
            }
            "app.hdr.peak" => {
                self.next_peak();
                let text = self.ctx.output.describe();
                self.console.print(text, Color::Green);
            }
            "app.hdr.false_colours" => match &mut self.hdr.preview {
                Some(preview) => {
                    preview.false_colours = !preview.false_colours;
                    let state = if preview.false_colours { "on" } else { "off" };
                    self.console
                        .print(format!("the preview's false colours {state}"), Color::Green);
                }
                None => self.console.print(
                    "no HDR preview (off-screen HDR only)".to_owned(),
                    Color::Red,
                ),
            },
            "app.calibrate" => {
                if self.ctx.output.is_hdr() {
                    self.open_calibration(Page::Peak)?;
                    self.console.set_open(false);
                    self.ctx.settings.set(self.console_shown, false);
                } else {
                    self.console.print(
                        "the calibration needs HDR: app.hdr first".to_owned(),
                        Color::Red,
                    );
                }
            }
            // The overlay's groups of zones, folded or shown (keys 1 to 9 until D-069).
            "app.profile.group" => match args.first().and_then(|a| a.parse::<usize>().ok()) {
                Some(group @ 1..=9) => {
                    self.ctx.profile.toggle_group(group - 1);
                    self.console.print(
                        format!("the overlay's group {group} folded or shown"),
                        Color::Green,
                    );
                }
                _ => self
                    .console
                    .print("app.profile.group N (1 to 9)".to_owned(), Color::Red),
            },
            _ => match self.demo.command(&mut self.ctx, name, args) {
                Some(result) => self.console.report(result),
                None => self
                    .console
                    .print(format!("{name}: nothing runs it in this demo"), Color::Red),
            },
        }
        Ok(false)
    }

    fn calibration_key(&mut self, code: KeyCode, fine: bool) -> Result<()> {
        let peak = self.ctx.output.peak;
        let action = self.hdr.calibration.key(code, fine, &mut self.ctx.output);
        if action == Action::None {
            return Ok(());
        }
        self.overlay.set_output(
            &self.ctx.shaders,
            self.ctx.swapchain.format(),
            self.ctx.output.ui_white,
        )?;
        self.content_light_changed(peak);
        if action == (Action::Closed { save: true }) {
            if let Some(file) = &self.settings_file
                && let Err(error) = file.save(&self.monitor, self.ctx.output.settings)
            {
                tracing::warn!(%error, "the HDR calibration was not saved");
            }
            tracing::info!("{}", self.ctx.output.describe());
        }
        Ok(())
    }

    fn new(
        window: Arc<Window>,
        config: AppConfig,
        shaders: Option<ShaderCompiler>,
        init: InitFn<D>,
    ) -> Result<Self> {
        // In the load profile (#345): the device's creation, Streamline's start inside it, which
        // a loading demo's preparation runs beside.
        let device_task = forge_core::activity::begin(
            "app",
            if config.streamline {
                "the device, Streamline starting"
            } else {
                "the device"
            },
        );
        let display = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();
        let validation = config.validate || cfg!(debug_assertions);
        let root = workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let options = DeviceOptions {
            no_mesh_shader: config.force_fallback,
        };
        // NVIDIA Streamline's interposer becomes the Vulkan loader, so it is loaded only when the
        // GPU the device selection prefers is NVIDIA's (issue #67): a plain instance asks first.
        // Any failure through Streamline (instance, surface or device) falls back to the plain
        // loader: DLSS is an option, TAA the floor.
        let streamlined = if config.streamline {
            let vendor = {
                let probe = Arc::new(Instance::new(c"forge", validation, Some(display))?);
                let surface = probe.create_surface(display, window_handle)?;
                Device::preferred_vendor(&probe, Some(surface.raw()))?
            };
            if vendor == Some(VENDOR_NVIDIA) {
                let sdk = std::env::var_os("FORGE_STREAMLINE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| root.join("streamline-sdk/bin/x64"));
                let through = || -> forge_gpu::Result<_> {
                    let instance = Arc::new(Instance::with_streamline(
                        c"forge",
                        validation,
                        Some(display),
                        &sdk,
                    )?);
                    let surface = instance.create_surface(display, window_handle)?;
                    let device =
                        Device::with_options(Arc::clone(&instance), Some(surface.raw()), options)?;
                    Ok((instance, surface, device))
                };
                match through() {
                    Ok(gpu) => Some(gpu),
                    Err(error) => {
                        tracing::warn!(%error, sdk = %sdk.display(), "no Streamline: DLSS unavailable");
                        None
                    }
                }
            } else {
                tracing::info!(vendor = ?vendor, "no Streamline: DLSS needs an NVIDIA GPU");
                None
            }
        } else {
            None
        };
        let (instance, surface, device) = match streamlined {
            Some(gpu) => gpu,
            None => {
                let instance = Arc::new(Instance::new(c"forge", validation, Some(display))?);
                let surface = instance.create_surface(display, window_handle)?;
                let device =
                    Device::with_options(Arc::clone(&instance), Some(surface.raw()), options)?;
                (instance, surface, device)
            }
        };
        let size = window.inner_size();
        let swapchain = Swapchain::new(
            Arc::clone(&device),
            Arc::clone(&surface),
            size.width,
            size.height,
            // Scripted runs measure their frames uncapped; interactive ones never tear.
            if config.vsync {
                Pacing::Vsync
            } else if config.frame_limit.is_some() {
                Pacing::Uncapped
            } else {
                Pacing::Smooth
            },
        )?;
        drop(device_task);
        // What the OS says of the window's display (issue #94), under the calibration saved for
        // it and the flags (#125). Scripted runs keep to the OS's values, so captures do not
        // depend on a calibration.
        let monitor = monitor_name(&window);
        let caps = monitor.as_deref().and_then(DisplayCaps::query);
        if let Some(caps) = &caps {
            tracing::info!(?caps, "display");
        }
        let monitor = monitor.unwrap_or_else(|| "default".to_owned());
        let settings_file = config
            .frame_limit
            .is_none()
            .then(|| SettingsFile::new(settings_dir(&root).join("display.txt")));
        let mut output = DisplayOutput::new(swapchain.format(), caps);
        if let Some(file) = &settings_file {
            output.settings = file.load(&monitor);
        }
        if config.hdr_ui_white.is_some() {
            output.settings.ui_white = config.hdr_ui_white;
        }
        output.scene_stops = config.hdr_stops;
        output.refresh(caps);
        let frames = Frames::new(Arc::clone(&device), swapchain.image_count())?;
        let shaders = match shaders {
            Some(shaders) => shaders,
            None => shader_compiler(&config)?,
        };
        let font_path = std::env::var_os("FORGE_OVERLAY_FONT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf"));
        let font_px = std::env::var("FORGE_OVERLAY_FONT_PX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(14.0);
        let mut overlay = Overlay::new(
            &device,
            &shaders,
            swapchain.format(),
            Some(&font_path),
            font_px,
        )?;
        // `FORGE_OVERLAY=off|compact|full` overrides the demo's choice (scripted captures of the overlay).
        let overlay_mode = match std::env::var("FORGE_OVERLAY").ok().as_deref() {
            Some("off") => OverlayMode::Off,
            Some("compact") => OverlayMode::Compact,
            Some("full") => OverlayMode::Full,
            _ if config.overlay.unwrap_or(false) => OverlayMode::Compact,
            _ => OverlayMode::Off,
        };
        let graph = RenderGraph::new(&device);
        // The settings (D-069): the file, then the command line, applied as each variable
        // registers; the shell's own first.
        let mut settings = Settings::new();
        // The user's file first (the menu's choices, the first launch's): `forge.cfg` and
        // `--set` override it. Never in a scripted run (`FORGE_CFG=none`, `--no-cfg`), which
        // reads no file at all.
        let user_file = config
            .settings
            .cfg_path()
            .is_some()
            .then(|| settings_dir(&root).join("user.cfg"));
        let mut user_header = String::new();
        if let Some(path) = &user_file {
            if path.exists() {
                user_header = std::fs::read_to_string(path)
                    .unwrap_or_default()
                    .lines()
                    .take_while(|l| l.starts_with('#'))
                    .map(|l| format!("{l}\n"))
                    .collect();
                match settings.load_file_as(path, settings::Source::User) {
                    Ok(n) => {
                        tracing::info!(path = %path.display(), lines = n, "user settings read")
                    }
                    Err(e) => tracing::warn!("user settings: {e}"),
                }
            } else if config.frame_limit.is_none() && config.capture.is_none() {
                // The first launch on this machine: what the GPU and the monitor are, and the
                // settings that fit them, written for the runs to come.
                let machine = presets::Machine {
                    video_memory: device
                        .memory_report()
                        .device_local()
                        .map(|(_, budget)| budget),
                    dlss: device.dlss().is_some(),
                    monitor: window.current_monitor().map(|m| {
                        let size = m.size();
                        (size.width, size.height)
                    }),
                };
                user_header = format!(
                    "{}\n",
                    presets::first_launch_header(&machine, device.name())
                );
                for (name, value) in presets::first_launch(&machine) {
                    if let Err(e) = settings.set_text(&name, &value, settings::Source::User) {
                        tracing::warn!("first launch: {name}: {e}");
                    }
                }
                tracing::info!(
                    ?machine,
                    gpu = device.name(),
                    "first launch: settings chosen"
                );
            }
        }
        if let Some(cfg) = config.settings.cfg_path() {
            match settings.load_file(&cfg) {
                Ok(0) => {}
                Ok(n) => tracing::info!(path = %cfg.display(), lines = n, "settings file read"),
                Err(e) => tracing::warn!("settings file: {e}"),
            }
        }
        for assignment in &config.settings.set {
            match settings::split_assignment(assignment) {
                Some((name, value)) => {
                    if let Err(e) = settings.set_text(name, value, settings::Source::CommandLine) {
                        tracing::warn!("--set {assignment}: {e}");
                    }
                }
                None => tracing::warn!("--set {assignment}: not NAME=VALUE"),
            }
        }
        let display_info = settings.add(
            Spec::int(
                "r.display_info",
                match overlay_mode {
                    OverlayMode::Off => 0,
                    OverlayMode::Compact => 1,
                    OverlayMode::Full => 2,
                },
                0..=2,
                "Frame times and statistics: 0 off, 1 on, 2 on (verbose). F1 steps through them.",
            )
            .release(),
        );
        let console_shown = settings.add(
            Spec::bool(
                "app.console",
                false,
                "The console, over the top of the window; the key under Esc toggles it.",
            )
            .release(),
        );
        let mouse_look = settings.add(
            Spec::bool(
                "app.mouse_look",
                false,
                "The game's mouse look: the cursor captured and hidden, the view turning without a \
                 button held, every mouse button the game's; Esc releases it.",
            )
            .release(),
        );
        let hud_shown = settings.add(
            Spec::bool(
                "app.hud",
                true,
                "The game's HUD over the picture (#328): off for a capture without it.",
            )
            .release(),
        );
        let preset = settings.add(
            Spec::choice(
                "r.preset",
                presets::PRESETS,
                presets::CUSTOM,
                "Quality preset: custom, low, medium, high or ultra. One sets how small DLSS and \
                 FSR render, the ambient occlusion, the mirror reflections, the local exposure \
                 and the detail's error; changing one of those by hand puts it back to custom. \
                 The first launch picks one from the GPU's memory.",
            )
            .release(),
        );
        let window_mode = settings.add(
            Spec::choice(
                "app.window",
                &["windowed", "fullscreen"],
                0,
                "The window, or borderless fullscreen on its monitor at the monitor's own size \
                 (Alt+Enter switches too). The first launch picks fullscreen.",
            )
            .release(),
        );
        let volume = settings.add(
            Spec::float(
                "snd.volume",
                1.0,
                0.0..=1.0,
                "The sound's master level, 0 to 1 (#318; 0 is silent).",
            )
            .release(),
        );
        let ambience = settings.add(
            Spec::float(
                "snd.ambience",
                1.0,
                0.0..=1.0,
                "The ambience's level under the master, 0 to 1 (#357): the wind, the birds, the \
                 cicadas, the sea.",
            )
            .release(),
        );
        // A load profiled (#345): from here to the demo's first frame, in the log and in
        // `cache/profiles/` (the latest few kept); compiled out of shipped builds.
        let load_profile = settings.add(
            Spec::bool(
                "dev.load_profile",
                false,
                "Profiles the start's load (#345): each step's time, its thread and the bytes it \
                 uploaded, in the log and in cache/profiles/ (the 10 latest kept). Read at start.",
            )
            .restart(),
        );
        // `run_loading` began recording with its preparation: kept when asked, else dropped.
        if settings.get(load_profile) {
            if !forge_core::activity::recording() {
                forge_core::activity::record_start();
            }
        } else if forge_core::activity::recording() {
            forge_core::activity::record_finish();
        }
        settings.add_command(
            "app.capture",
            "[PATH]",
            "Saves the next frame as a PNG (capture.png by default).",
            false,
        );
        settings.add_command("app.quit", "", "Closes the window.", true);
        settings.add_command(
            "app.menu",
            "",
            "Opens the game's menu, as Esc does (#352).",
            true,
        );
        settings.add_command(
            "app.fullscreen",
            "",
            "Borderless fullscreen on the window's monitor, or back to the window as it was \n             (Alt+Enter).",
            true,
        );
        settings.add_command(
            "app.hdr",
            "",
            "The HDR output on or off: the mode asked for at start, else HDR10 when the OS shows \
             the display in HDR, else off-screen.",
            false,
        );
        settings.add_command("app.hdr.peak", "", "The next of ACES 2.0's peaks.", false);
        settings.add_command(
            "app.hdr.false_colours",
            "",
            "The off-screen HDR preview's false colours on or off.",
            false,
        );
        settings.add_command(
            "app.calibrate",
            "",
            "The HDR calibration pages (in an HDR mode).",
            false,
        );
        settings.add_command(
            "app.profile.group",
            "N",
            "Folds or shows the overlay's group N of zones (1 to 9, in r.display_info 2).",
            false,
        );
        let mut ctx = Context {
            device,
            swapchain,
            output,
            frames,
            shaders,
            window,
            frames_rendered: 0,
            captures_written: 0,
            profile: Profile::new(overlay_mode),
            graph,
            capture_request: None,
            loading: false,
            preparing: false,
            settings,
            sound: None,
            hud: Vec::new(),
            magnifier: None,
            ssaa: 1,
            paused: false,
            glyphs: None,
            font_path: font_path.clone(),
            counted_from: 0,
            _surface: surface,
            _instance: instance,
        };
        let requested = match std::env::var("FORGE_HDR") {
            Ok(text) => text.parse().unwrap_or_else(|error| {
                tracing::warn!("FORGE_HDR: {error}");
                config.hdr
            }),
            Err(_) => config.hdr,
        };
        let mut hdr = HdrPasses {
            preview: None,
            meter: None,
            calibration: Calibration::new(),
            ssaa: None,
        };
        apply_hdr_mode(&mut ctx, &mut overlay, &mut hdr, requested)?;
        if config.ssaa {
            if ctx.output.mode == HdrMode::Offscreen {
                tracing::warn!("no supersampling in the off-screen HDR mode");
            } else {
                ctx.ssaa = ssaa::FACTOR;
                hdr.ssaa = Some(ssaa::Ssaa::new(
                    &ctx.device,
                    &ctx.shaders,
                    ctx.swapchain.format(),
                )?);
                tracing::info!(extent = ?ctx.extent(), "supersampling 2 × 2");
            }
        }
        // `FORGE_HDR_CALIBRATION=peak|black|white` opens a calibration page (scripted captures).
        if let Ok(name) = std::env::var("FORGE_HDR_CALIBRATION") {
            match Page::parse(&name) {
                Some(page) if ctx.output.is_hdr() => {
                    hdr.calibration
                        .open(&ctx.device, &ctx.shaders, &ctx.output, page)?;
                }
                Some(_) => tracing::warn!("FORGE_HDR_CALIBRATION needs an HDR mode"),
                None => tracing::warn!("FORGE_HDR_CALIBRATION: peak, black or white"),
            }
        }
        // Sound before the demo starts, so that it can play from its first frame (#318).
        let mut sound_recorder = None;
        let sound_output = if let Some(path) = config.sound_record.clone().filter(|_| config.sound)
        {
            let (sound, recorder) = sound::Recorder::start(path, config.sound_record_step);
            ctx.sound = Some(sound);
            sound_recorder = Some(recorder);
            None
        } else if sound::wanted(&config) {
            sound::open().map(|(sound, output)| {
                ctx.sound = Some(sound);
                output
            })
        } else {
            None
        };
        let mut demo = init(&mut ctx)?;
        demo.register(&mut ctx);
        // The history is kept between interactive runs only: scripted ones never open it.
        let console = console::Console::new(
            config
                .frame_limit
                .is_none()
                .then(console::history_path)
                .flatten(),
        );
        Ok(Self {
            console,
            modifiers: winit::keyboard::ModifiersState::default(),
            windowed: None,
            display_info,
            console_shown,
            mouse_look,
            hud: hud::Hud::new(&ctx.device, &ctx.shaders, ctx.swapchain.format())?,
            hud_shown,
            volume,
            volume_sent: 1.0,
            ambience,
            ambience_sent: 1.0,
            preset,
            preset_applied: 0,
            window_mode,
            user_file,
            user_header,
            menu: menu::Menu::default(),
            cursor: [0.0; 2],
            allocs: alloc_count::PerFrame::default(),
            _sound_output: sound_output,
            sound_recorder,
            unmatched_reported: false,
            demo,
            ctx,
            hdr,
            hdr_requested: requested,
            settings_file,
            monitor,
            input: Input::default(),
            needs_resize: None,
            stale_swapchain: false,
            last_frame: Instant::now(),
            last_title: Instant::now(),
            config,
            debug_wait_idle: std::env::var_os("FORGE_WAIT_IDLE").is_some_and(|v| v != "0"),
            shown: false,
            debug_no_title: std::env::var_os("FORGE_NO_TITLE").is_some_and(|v| v != "0"),
            debug_stall_ms: std::env::var("FORGE_STALL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            debug_hdr_cycle: std::env::var("FORGE_HDR_CYCLE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            last_hdr_cycle: 0,
            loading_capture: std::env::var_os("FORGE_LOADING_CAPTURE").map(|path| {
                let at = std::env::var("FORGE_LOADING_CAPTURE_AT")
                    .ok()
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(2.0);
                (
                    PathBuf::from(path),
                    Instant::now() + Duration::from_secs_f64(at.max(0.0)),
                )
            }),
            overlay,
            graph_stats: GraphStats::default(),
            memory_mark: None,
            memory_start: None,
            #[cfg(feature = "profiling")]
            tracy_gpu: None,
        })
    }

    /// Mirrors the frame's GPU zones into Tracy's GPU timeline (`--features profiling`).
    #[cfg(feature = "profiling")]
    fn tracy_gpu_zones(&mut self) {
        let zones = self.ctx.frames.gpu_zones();
        if zones.is_empty() {
            return;
        }
        if self.tracy_gpu.is_none()
            && let Some(client) = tracy_client::Client::running()
        {
            let period = self.ctx.device.timestamp_period_ns();
            self.tracy_gpu = client
                .new_gpu_context(
                    Some("GPU"),
                    tracy_client::GpuContextType::Vulkan,
                    zones[0].start_ticks as i64,
                    period,
                )
                .ok();
        }
        if let Some(context) = &self.tracy_gpu {
            for zone in zones {
                if let Ok(mut span) = context.span_alloc(zone.label, "gpu", "gpu", 0) {
                    span.upload_timestamp_start(zone.start_ticks as i64);
                    span.end_zone();
                    span.upload_timestamp_end(zone.end_ticks as i64);
                }
            }
        }
    }

    fn frame(&mut self) -> Result<()> {
        if self.debug_hdr_cycle > 0
            && !self.ctx.loading
            && self.ctx.frames_rendered > 0
            && self
                .ctx
                .frames_rendered
                .is_multiple_of(self.debug_hdr_cycle)
            && self.ctx.frames_rendered != self.last_hdr_cycle
        {
            self.last_hdr_cycle = self.ctx.frames_rendered;
            self.toggle_hdr()?;
        }
        // The demo's preparation on the GPU (`Context::preparing`: its impostors' bakes, about
        // 130 frames a kind) presented uncapped: at the display's refresh the village's 81 kinds
        // took about 85 s behind the screen (2026-10-29). Smooth again once it is over. The
        // same size, so the demo's targets stay, unless the window changed meanwhile.
        if self.ctx.swapchain.set_uncapped_for_now(self.ctx.preparing) {
            if self.needs_resize.is_some() {
                self.stale_swapchain = true;
            } else {
                let before = self.ctx.swapchain.extent();
                self.ctx.swapchain.recreate(before.width, before.height)?;
                self.ctx
                    .frames
                    .resize_swapchain(self.ctx.swapchain.image_count())?;
                if self.ctx.swapchain.extent() != before {
                    self.demo.resized(&mut self.ctx)?;
                }
            }
        }
        if let Some(size) = self.needs_resize.take() {
            if size.width == 0 || size.height == 0 {
                self.needs_resize = Some(size);
                return Ok(());
            }
            let current = self.ctx.swapchain.extent();
            if self.stale_swapchain || size.width != current.width || size.height != current.height
            {
                self.stale_swapchain = false;
                self.ctx.swapchain.recreate(size.width, size.height)?;
                self.ctx
                    .frames
                    .resize_swapchain(self.ctx.swapchain.image_count())?;
                tracing::info!(
                    frame = self.ctx.frames_rendered,
                    width = size.width,
                    height = size.height,
                    "swapchain resized"
                );
                self.demo.resized(&mut self.ctx)?;
            }
        }
        let now = Instant::now();
        let frame_ms = (now - self.last_frame).as_secs_f64() * 1e3;
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.ctx.profile.begin_frame();
        self.ctx.profile.frame_time(frame_ms);
        let g = self.graph_stats;
        self.ctx.profile.counter(format!(
            "graph: {} passes, {} image + {} memory barriers; transients {} images + {} buffers, {:.1} MB in a {:.1} MB heap (load {:.1} MB){}; {} rebuilds, {} retired",
            g.passes,
            g.image_barriers,
            g.memory_barriers,
            g.transient_images,
            g.transient_buffers,
            g.transient_bytes as f64 / 1e6,
            g.heap_bytes as f64 / 1e6,
            g.load_bytes as f64 / 1e6,
            if g.aliased { " (aliased)" } else { "" },
            g.heap_rebuilds,
            g.pending_destructions
        ));
        // `r.display_info` (D-069) is what the overlay shows; F1 and the console both set it.
        let wanted = match self.ctx.settings.get(self.display_info) {
            0 => OverlayMode::Off,
            1 => OverlayMode::Compact,
            _ => OverlayMode::Full,
        };
        self.ctx.profile.set_mode(wanted);
        // `r.preset` and `app.window` followed, and the user's file written when the menu (or a
        // preset chosen as the user's) changed a setting.
        self.apply_preset();
        self.follow_window_mode();
        if self.ctx.settings.take_user_changed() {
            self.write_user_file();
        }
        // `app.mouse_look` (D-069): the game's mouse look, the cursor captured (locked where the
        // platform can, else kept in the window) and hidden, the view turning with no button.
        let look = self.ctx.settings.get(self.mouse_look);
        if look != self.input.mouse_look {
            self.input.mouse_look = look;
            self.input.paused = self.menu.is_open();
            // Out of the window, it waits for the focus to come back (`WindowEvent::Focused`).
            let take = look && !self.input.unfocused && !self.menu.is_open();
            self.input.looking = take;
            if take {
                capture_cursor(&self.ctx.window);
            } else {
                free_cursor(&self.ctx.window);
            }
        }
        if !self.unmatched_reported && !self.ctx.loading {
            self.unmatched_reported = true;
            let unmatched: Vec<(String, settings::Source)> = self
                .ctx
                .settings
                .unmatched()
                .map(|(name, source)| (name.to_owned(), source))
                .collect();
            for (name, source) in &unmatched {
                let text = format!(
                    "{name} ({}): no such setting (the console's `find` lists them)",
                    source.describe()
                );
                tracing::warn!("{text}");
                self.console.print(text, Color::Red);
            }
            // A scripted run fails on a misspelled `--set`, rather than capture the wrong thing.
            if self.config.frame_limit.is_some()
                && let Some((name, _)) = unmatched
                    .iter()
                    .find(|(_, s)| *s == settings::Source::CommandLine)
            {
                anyhow::bail!("--set {name}: no such setting in this demo");
            }
            // `forge.cfg`'s binds, then `--exec`: console lines run once the demo is ready, as
            // if typed.
            let mut lines = self.ctx.settings.take_startup();
            lines.extend(self.config.settings.exec.clone());
            for line in lines {
                self.console.print(format!("> {line}"), Color::Dim);
                if let Some(console::Request::Command { name, args }) =
                    self.console.run(&line, &mut self.ctx.settings)
                    && self.run_command(&name, &args)?
                {
                    // `app.quit`: this frame is the last.
                    self.config.frame_limit = Some(self.ctx.frames_rendered);
                }
            }
        }
        let shown = self.ctx.settings.get(self.console_shown) && !self.ctx.loading;
        if shown != self.console.is_open() {
            self.console.set_open(shown);
            if shown {
                self.input.release_all();
            }
        }
        let was_loading = self.ctx.loading;
        // Behind the game's menu a solo game holds (#352): its update sees no time pass, and its
        // sound fades out over a quarter of a second, and back in after.
        self.ctx.paused = self.menu.is_open() && !self.demo.networked();
        if self.menu.is_open() {
            self.input.mouse_delta = (0.0, 0.0);
            self.input.wheel = 0.0;
        }
        if let Some(sound) = self.ctx.sound.as_mut() {
            sound.update();
            let volume = if self.ctx.paused {
                0.0
            } else {
                self.ctx.settings.get(self.volume)
            };
            let fade = f64::from(dt) * 4.0;
            let gain = self.volume_sent + (volume - self.volume_sent).clamp(-fade, fade);
            let gain = if (gain - volume).abs() < 1e-3 {
                volume
            } else {
                gain
            };
            if gain != self.volume_sent {
                sound.engine.control(forge_audio::Control::BusGain {
                    bus: forge_audio::BusId::MASTER,
                    gain: gain as f32,
                });
                self.volume_sent = gain;
            }
            let ambience = self.ctx.settings.get(self.ambience);
            if ambience != self.ambience_sent {
                sound.engine.control(forge_audio::Control::BusGain {
                    bus: sound.buses.ambience.beds,
                    gain: ambience as f32,
                });
                self.ambience_sent = ambience;
            }
        }
        let dt = if self.ctx.paused { 0.0 } else { dt };
        let update_start = Instant::now();
        {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("update");
            self.demo.update(&mut self.ctx, &self.input, dt);
        }
        if was_loading && !self.ctx.loading {
            // The demo started in this update, behind the loading screen (issue #25): its
            // start-up is neither the update's time nor the next frame's.
            self.last_frame = Instant::now();
        } else {
            self.ctx
                .profile
                .cpu_zone("cpu/update", ms_since(update_start));
        }
        self.input.end_frame();

        let wait_start = Instant::now();
        let slot = {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("wait for frame slot");
            self.ctx.frames.wait_for_slot()?
        };
        self.ctx
            .profile
            .cpu_zone("cpu/wait for GPU (frame slot)", ms_since(wait_start));
        // The zones are those of the frame that used the slot before, FRAMES_IN_FLIGHT ago.
        if self.ctx.loading || self.ctx.preparing {
            self.ctx.counted_from = slot.frame_number + 1;
        } else if slot.frame_number >= self.ctx.counted_from + FRAMES_IN_FLIGHT as u64 {
            self.ctx
                .profile
                .gpu_zones(self.ctx.frames.gpu_zones(), slot.previous_gpu_ms);
        }
        #[cfg(feature = "profiling")]
        self.tracy_gpu_zones();
        #[cfg(feature = "profiling")]
        if let Some(ms) = slot.previous_gpu_ms {
            tracy_client::plot!("gpu ms", ms);
        }
        // The content light the slot's last frame measured (#125): the display hears of it
        // when it grows.
        if let Some((measured, light)) = self.hdr.meter.as_mut().and_then(|m| m.take(slot)) {
            tracing::debug!(frame = measured, ?light, "content light");
            let grew = match &mut self.ctx.output.content_light {
                Some(content) => content.grow(light),
                None => {
                    self.ctx.output.content_light = Some(light);
                    true
                }
            };
            if grew {
                self.ctx
                    .swapchain
                    .set_hdr_metadata(self.ctx.output.metadata());
            }
        }
        let acquire_start = Instant::now();
        let Some(image_index) = self
            .ctx
            .swapchain
            .acquire(self.ctx.frames.image_available(slot))?
        else {
            self.needs_resize = Some(self.ctx.window.inner_size());
            self.stale_swapchain = true;
            return Ok(());
        };
        self.ctx
            .profile
            .cpu_zone("cpu/acquire swapchain image", ms_since(acquire_start));
        let extent = self.ctx.swapchain.extent();
        let frame_number = self.ctx.frames_rendered;
        let capture_path = match &self.config.capture {
            _ if self.ctx.loading || self.ctx.preparing => self
                .loading_capture
                .take_if(|(_, at)| Instant::now() >= *at)
                .map(|(path, _)| path),
            Some((path, frame)) if *frame == frame_number => Some(path.clone()),
            Some((path, _))
                if self
                    .config
                    .capture_every
                    .is_some_and(|n| n > 0 && frame_number.is_multiple_of(n)) =>
            {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "capture".to_owned());
                Some(path.with_file_name(format!("{stem}-{frame_number:05}.png")))
            }
            _ => None,
        };
        // The demo's own request, taken by this frame.
        let capture_path = match capture_path {
            None if !self.ctx.loading && !self.ctx.preparing => self.ctx.capture_request.take(),
            path => path,
        };
        // Four times per second, and on a captured frame so that a scripted capture shows
        // current counters however fast the frames went by.
        if !self.ctx.loading
            && (capture_path.is_some()
                || self
                    .memory_mark
                    .is_none_or(|mark| mark.at.elapsed() >= MEMORY_SAMPLE_PERIOD))
        {
            self.sample_memory_since(self.memory_mark);
        }
        // In the off-screen HDR mode the frame is drawn into an HDR10 image (issue #94).
        let offscreen = self.ctx.output.mode == HdrMode::Offscreen && !self.ctx.loading;
        let capture_buffer = |name: &'static str, format: vk::Format| {
            let bytes_per_pixel = if format == vk::Format::R16G16B16A16_SFLOAT {
                8
            } else {
                4
            };
            self.ctx
                .device
                .create_buffer(BufferDesc {
                    size: u64::from(extent.width) * u64::from(extent.height) * bytes_per_pixel,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name,
                })
                .map(GraphBuffer::new)
        };
        let capture = capture_path
            .as_ref()
            .map(|path| {
                Ok::<_, forge_gpu::GpuError>((
                    path.clone(),
                    capture_buffer("capture", self.ctx.swapchain.format())?,
                ))
            })
            .transpose()?;
        // Beside the preview, the HDR image's PQ codes (`-pq.png`, 16 bits).
        let capture_hdr = capture_path
            .filter(|_| offscreen)
            .map(|path| {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "capture".to_owned());
                Ok::<_, forge_gpu::GpuError>((
                    path.with_file_name(format!("{stem}-pq.png")),
                    capture_buffer("capture hdr", OFFSCREEN_FORMAT)?,
                ))
            })
            .transpose()?;

        // The window's format changes with HDR (`app.hdr`): supersampling's pass follows it.
        if let Some(pass) = &self.hdr.ssaa
            && pass.format() != self.ctx.swapchain.format()
        {
            self.ctx.device.wait_idle();
            self.hdr.ssaa = Some(ssaa::Ssaa::new(
                &self.ctx.device,
                &self.ctx.shaders,
                self.ctx.swapchain.format(),
            )?);
        }
        self.ctx.frames.begin(slot)?;
        let record_start = Instant::now();
        {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("record");
            let mut graph = FrameGraph::new(extent);
            // The acquired swapchain image: contents undefined, usable once the acquire
            // semaphore's stage (colour output) has passed.
            let swapchain_image = graph.import_raw(RawImage {
                image: self.ctx.swapchain.image(image_index),
                view: self.ctx.swapchain.view(image_index),
                extent,
                format: self.ctx.swapchain.format(),
                aspect: vk::ImageAspectFlags::COLOR,
                state: ResourceState {
                    layout: vk::ImageLayout::UNDEFINED,
                    stage: vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT,
                    access: vk::AccessFlags2::NONE,
                    write: false,
                },
                sampled: self.ctx.swapchain.sampled(image_index),
                name: "swapchain",
            });
            let target = if offscreen {
                graph.transient(TransientDesc {
                    name: "hdr target",
                    width: extent.width,
                    height: extent.height,
                    format: OFFSCREEN_FORMAT,
                    usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::TRANSFER_SRC,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                })
            } else if self.hdr.ssaa.is_some() && !self.ctx.loading {
                let large = self.ctx.extent();
                graph.transient(TransientDesc {
                    name: "ssaa target",
                    width: large.width,
                    height: large.height,
                    format: self.ctx.swapchain.format(),
                    usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                })
            } else {
                swapchain_image
            };
            let mut frame = FrameInfo {
                graph,
                target,
                slot,
                image_index,
                dt,
            };
            self.demo.render(&mut self.ctx, &mut frame)?;
            // Supersampled: the large frame filtered down to the window, which the rest takes.
            let target = match &self.hdr.ssaa {
                Some(pass) if target != swapchain_image => {
                    pass.draw(&mut frame.graph, target, swapchain_image, extent);
                    swapchain_image
                }
                _ => target,
            };
            // In HDR (#125): a calibration page over the frame, or the content light of the
            // frame as shown (when its image can be sampled: always off-screen, on the display
            // when the surface allows it).
            let hdr_frame = self.ctx.output.is_hdr() && !self.ctx.loading;
            let calibrating = hdr_frame && self.hdr.calibration.page().is_some();
            if calibrating {
                self.hdr
                    .calibration
                    .draw(&mut frame.graph, target, extent, &self.ctx.output);
            } else if hdr_frame
                && (offscreen || self.ctx.swapchain.sampled(image_index).is_some())
                && let Some(meter) = &mut self.hdr.meter
            {
                // `OUTPUT_SCRGB` or `OUTPUT_PQ` of `tonemap.slang`.
                let encoding = if self.ctx.output.format == vk::Format::R16G16B16A16_SFLOAT {
                    3
                } else {
                    2
                };
                meter.measure(
                    &mut frame.graph,
                    slot,
                    frame_number,
                    target,
                    extent,
                    encoding,
                );
            }
            if offscreen && let Some(preview) = &self.hdr.preview {
                preview.draw(
                    &mut frame.graph,
                    target,
                    swapchain_image,
                    extent,
                    &self.ctx.output,
                );
            }
            // The game's HUD (#328): over the picture, under the debug overlay; its lens first
            // (not in the HDR outputs yet), then its quads.
            // The game's menu (#352) over them, shown with `app.hud 0` too.
            let hud_shown = self.ctx.settings.get(self.hud_shown);
            let lens = self
                .ctx
                .magnifier
                .take()
                .filter(|_| hud_shown && !self.ctx.output.is_hdr());
            if !hud_shown {
                self.ctx.hud.clear();
            }
            if self.menu.is_open()
                && let Some(Some(font)) = &self.ctx.glyphs
            {
                let size = [extent.width as f32, extent.height as f32];
                let paused = self.ctx.paused;
                self.menu
                    .draw(font, size, &self.ctx.settings, paused, &mut self.ctx.hud);
            }
            if !self.ctx.loading && (!self.ctx.hud.is_empty() || lens.is_some()) {
                self.hud.set_output(
                    &self.ctx.shaders,
                    self.ctx.swapchain.format(),
                    self.ctx.output.ui_white,
                )?;
                if let Some(lens) = lens {
                    self.hud
                        .draw_magnifier(&mut frame.graph, swapchain_image, extent, lens);
                }
                self.hud.draw(
                    &mut frame.graph,
                    slot.index,
                    swapchain_image,
                    extent,
                    &self.ctx.hud,
                );
            }
            self.ctx.hud.clear();
            let profiling = self.ctx.profile.is_visible() && !self.ctx.loading;
            let console = self.console.is_open() && !self.ctx.loading;
            // What a bound key did, a few seconds at the bottom (D-069).
            let now = Instant::now();
            let notices = !console && !self.ctx.loading && self.console.has_notices(now);
            // Over the loading screen: what the engine is busy with (`forge_core::activity`), and
            // over the frames the demo prepares in (`Context::preparing`: black, under the screen
            // in an interactive run), its impostors' bakes counted.
            let loading = self.ctx.loading || self.ctx.preparing;
            if calibrating || profiling || console || notices || loading {
                if !calibrating && self.ctx.output.is_hdr() {
                    self.ctx.profile.counter(self.ctx.output.describe());
                }
                let title = self.config.title.clone();
                let canvas = self.overlay.begin(extent);
                if calibrating {
                    self.hdr.calibration.layout(canvas, &self.ctx.output);
                } else {
                    if loading {
                        loading::layout_activity(canvas, extent);
                    }
                    if profiling {
                        self.ctx.profile.layout(canvas, &title, extent);
                    }
                    // The console over the profile's top (D-069).
                    if console {
                        self.console.layout(canvas, &self.ctx.settings);
                    } else if notices {
                        self.console.layout_notices(canvas, now);
                    }
                }
                self.overlay
                    .draw(&mut frame.graph, slot.index, swapchain_image, extent);
            }
            for (image, capture) in [(swapchain_image, &capture), (target, &capture_hdr)] {
                let Some((_, buffer)) = capture else {
                    continue;
                };
                // The copy, then the host read after the wait below: declared, so the
                // device's writes are made visible to the host.
                let handle = frame.graph.import_buffer(buffer);
                frame
                    .graph
                    .pass("app/capture")
                    .image(image, ImageAccess::TransferSrc)
                    .buffer(handle, BufferAccess::TransferDst)
                    .run(move |resources, commands| {
                        commands.copy_image_to_buffer(resources.image(image).raw, extent, buffer);
                        Ok(())
                    });
                frame
                    .graph
                    .pass("app/capture")
                    .buffer(handle, BufferAccess::HostRead)
                    .run(|_, _| Ok(()));
            }
            frame
                .graph
                .pass("app/present")
                .image(swapchain_image, ImageAccess::Present)
                .run(|_, _| Ok(()));
            // What the demo and the overlay declared, then the graph's compile and its
            // recording (#78: the split the gate for parallel recording reads).
            self.ctx
                .profile
                .cpu_zone("cpu/declare passes", ms_since(record_start));
            // `FORGE_FRAME_BARRIER=1` (serialise frames on the GPU) lives in the graph now.
            self.graph_stats = self
                .ctx
                .graph
                .execute(frame.graph, &mut self.ctx.frames, slot)?;
            self.ctx
                .profile
                .cpu_zone("cpu/graph compile", f64::from(self.graph_stats.compile_ms));
            self.ctx
                .profile
                .cpu_zone("cpu/graph record", f64::from(self.graph_stats.record_ms));
        }
        let submit_start = Instant::now();
        {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("submit and present");
            self.ctx.frames.submit(slot, image_index)?;
            if self
                .ctx
                .swapchain
                .present(self.ctx.frames.render_finished(image_index), image_index)?
            {
                self.needs_resize = Some(self.ctx.window.inner_size());
            }
        }
        self.ctx
            .profile
            .cpu_zone("cpu/submit + present", ms_since(submit_start));
        #[cfg(feature = "profiling")]
        tracy_client::frame_mark();
        if self.debug_wait_idle {
            // Debugging aid (`FORGE_WAIT_IDLE=1`): no CPU/GPU overlap at all.
            self.ctx.device.wait_idle();
        }
        let capture_path_saved = capture.is_some();
        for (capture, format) in [
            (capture, self.ctx.swapchain.format()),
            (capture_hdr, OFFSCREEN_FORMAT),
        ] {
            if let Some((path, buffer)) = capture {
                self.ctx.device.wait_idle();
                save_capture(&path, &buffer, extent, format)?;
                tracing::info!(path = %path.display(), "captured frame {}", self.ctx.frames_rendered);
            }
        }
        if capture_path_saved {
            self.ctx.captures_written += 1;
        }
        if !self.ctx.loading && !self.ctx.preparing {
            self.ctx.frames_rendered += 1;
            self.allocs.end_frame(self.ctx.frames_rendered);
        }
        if let Some(recorder) = &mut self.sound_recorder {
            recorder.advance(dt);
        }
        if self.debug_stall_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.debug_stall_ms));
        }
        if !self.debug_no_title && self.last_title.elapsed() >= Duration::from_millis(250) {
            self.last_title = Instant::now();
            if let Some(title) = self.demo.title(&self.ctx) {
                self.ctx.window.set_title(&title);
            }
        }
        Ok(())
    }
}

fn save_capture(
    path: &PathBuf,
    buffer: &Buffer,
    extent: vk::Extent2D,
    format: vk::Format,
) -> Result<()> {
    if format == vk::Format::R16G16B16A16_SFLOAT {
        tracing::warn!(path = %path.display(), "scRGB frames are not captured: use --hdr offscreen");
        return Ok(());
    }
    let mut pixels = vec![0_u8; (extent.width * extent.height * 4) as usize];
    buffer.read(0, &mut pixels);
    if format == vk::Format::A2B10G10R10_UNORM_PACK32
        || format == vk::Format::A2R10G10B10_UNORM_PACK32
    {
        // HDR10: the 10-bit PQ codes in a 16-bit PNG (issue #94).
        let mut rgb = hdr::a2b10g10r10_to_rgb16(&pixels);
        if format == vk::Format::A2R10G10B10_UNORM_PACK32 {
            for px in rgb.as_chunks_mut::<3>().0 {
                px.swap(0, 2);
            }
        }
        image::ImageBuffer::<image::Rgb<u16>, _>::from_raw(extent.width, extent.height, rgb)
            .ok_or_else(|| anyhow::anyhow!("capture size"))?
            .save(path)?;
        return Ok(());
    }
    if format == vk::Format::B8G8R8A8_SRGB || format == vk::Format::B8G8R8A8_UNORM {
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
    }
    image::save_buffer(
        path,
        &pixels,
        extent.width,
        extent.height,
        image::ColorType::Rgba8,
    )?;
    Ok(())
}

struct App<D: Demo> {
    config: AppConfig,
    init: Option<InitFn<D>>,
    /// The shader compiler `run_loading` made at the process's start, for the shell to take.
    shaders: Option<ShaderCompiler>,
    state: Option<State<D>>,
    error: Option<anyhow::Error>,
}

/// The game's mouse look takes the cursor: locked where the platform can, else kept in the
/// window, and hidden.
fn capture_cursor(window: &Window) {
    if window.set_cursor_grab(CursorGrabMode::Locked).is_err() {
        let _ = window.set_cursor_grab(CursorGrabMode::Confined);
    }
    window.set_cursor_visible(false);
}

/// The cursor is the system's again: free and shown.
fn free_cursor(window: &Window) {
    let _ = window.set_cursor_grab(CursorGrabMode::None);
    window.set_cursor_visible(true);
}

impl<D: Demo> ApplicationHandler for App<D> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        // Hidden until its first frame is presented (#219): a new window shows the system's
        // white background until then, through the device's and swapchain's setup.
        let mut attributes = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(PhysicalSize::new(self.config.width, self.config.height))
            .with_visible(false);
        // Which monitor: `FORGE_MONITOR` = `secondary` (default: the first non-primary monitor,
        // so demos stay off the owner's working screen), `primary`, or an index into the
        // monitor list. Scripted runs (a frame limit) do not take keyboard focus.
        let preference = std::env::var("FORGE_MONITOR").unwrap_or_else(|_| "secondary".to_owned());
        let primary = event_loop.primary_monitor();
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        let chosen = match preference.as_str() {
            "primary" => primary.clone(),
            "secondary" => monitors
                .iter()
                .find(|m| Some(*m) != primary.as_ref())
                .cloned()
                .or_else(|| primary.clone()),
            index => index
                .parse::<usize>()
                .ok()
                .and_then(|i| monitors.get(i).cloned())
                .or_else(|| primary.clone()),
        };
        if let Some(monitor) = &chosen {
            let origin = monitor.position();
            attributes =
                attributes.with_position(PhysicalPosition::new(origin.x + 40, origin.y + 60));
            tracing::info!(monitor = ?monitor.name(), x = origin.x, y = origin.y, monitors = monitors.len(), "window placed");
        }
        let inactive = self.config.frame_limit.is_some();
        if inactive {
            attributes = attributes.with_active(false);
        }
        let Some(init) = self.init.take() else {
            return;
        };
        match event_loop.create_window(attributes) {
            Ok(window) => match State::new(
                Arc::new(window),
                self.config.clone(),
                self.shaders.take(),
                init,
            ) {
                Ok(mut state) => {
                    // A window made inactive never hears `Focused(true)` unless clicked: it starts
                    // out of focus, so `app.mouse_look` on the command line doesn't capture the
                    // owner's cursor during a scripted run.
                    state.input.unfocused = inactive;
                    self.state = Some(state);
                }
                Err(e) => {
                    self.error = Some(e);
                    event_loop.exit();
                }
            },
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.needs_resize = Some(size),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    // Alt+Enter, first of all (the console open or not): borderless fullscreen
                    // on the window's monitor, and back to the window as it was (the owner).
                    // AltGr (Ctrl+Alt on Windows) is not Alt. The demos never see that Enter.
                    if state.modifiers.alt_key()
                        && !state.modifiers.control_key()
                        && matches!(code, KeyCode::Enter | KeyCode::NumpadEnter)
                    {
                        if pressed && !event.repeat {
                            state.toggle_fullscreen();
                        }
                        return;
                    }
                    // The open calibration pages take the keys (#125); Escape leaves them.
                    if state.hdr.calibration.page().is_some() {
                        // Shift is tracked for the steps, and releases always (a key held
                        // when the pages opened must not stay down for the demo).
                        if !pressed || matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
                            state.input.set_key(code, pressed);
                        }
                        let fine = state.input.is_down(KeyCode::ShiftLeft)
                            || state.input.is_down(KeyCode::ShiftRight);
                        // Held, Up and Down repeat; the other keys act once.
                        let steps = matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown);
                        if pressed
                            && (steps || !event.repeat)
                            && let Err(e) = state.calibration_key(code, fine)
                        {
                            self.error = Some(e);
                            event_loop.exit();
                        }
                        return;
                    }
                    // The console (D-069): the key under Esc (² on AZERTY, ` on QWERTY) opens
                    // and closes it; open, it takes every key, and the demo sees only releases.
                    if pressed && code == KeyCode::Backquote && !state.ctx.loading {
                        let open = !state.console.is_open();
                        state.ctx.settings.set(state.console_shown, open);
                        state.console.set_open(open);
                        if open {
                            state.input.release_all();
                        }
                        return;
                    }
                    if state.console.is_open() {
                        if !pressed {
                            state.input.set_key(code, false);
                            return;
                        }
                        // Ctrl without Alt: AltGr (Ctrl+Alt on Windows) types `~ # { [ | \ @ ]`
                        // on AZERTY.
                        let ctrl = state.modifiers.control_key() && !state.modifiers.alt_key();
                        let request = state.console.key(
                            code,
                            event.text.as_deref(),
                            ctrl,
                            state.modifiers.shift_key(),
                            &mut state.ctx.settings,
                        );
                        match request {
                            Some(console::Request::Close) => {
                                state.ctx.settings.set(state.console_shown, false);
                                state.console.set_open(false);
                            }
                            Some(console::Request::Command { name, args }) => {
                                match state.run_command(&name, &args) {
                                    Ok(false) => {}
                                    Ok(true) => event_loop.exit(),
                                    Err(e) => {
                                        self.error = Some(e);
                                        event_loop.exit();
                                    }
                                }
                            }
                            None => {}
                        }
                        return;
                    }
                    // The game's menu (#352): open, it takes the keys (but F1), the demo sees only
                    // releases; held, the arrows repeat.
                    if state.menu.is_open() && code != KeyCode::F1 {
                        if !pressed {
                            state.input.set_key(code, false);
                        } else if !event.repeat
                            || matches!(
                                code,
                                KeyCode::ArrowUp
                                    | KeyCode::ArrowDown
                                    | KeyCode::ArrowLeft
                                    | KeyCode::ArrowRight
                            )
                        {
                            let action = state.menu.key(code, &mut state.ctx.settings);
                            if state.menu_action(action) {
                                event_loop.exit();
                            }
                        }
                        return;
                    }
                    // Esc opens it; while loading, Esc still closes the window.
                    if pressed && code == KeyCode::Escape {
                        if state.ctx.loading {
                            event_loop.exit();
                        } else if !event.repeat {
                            state.open_menu();
                        }
                        return;
                    }
                    if pressed && code == KeyCode::F1 {
                        let next = (state.ctx.settings.get(state.display_info) + 1) % 3;
                        state.ctx.settings.set(state.display_info, next);
                        return;
                    }
                    // A key bound to a console line (`bind`, D-069): the developer's switches are
                    // settings and commands, on the keys the owner binds; the others are the
                    // game's.
                    if pressed
                        && !event.repeat
                        && !state.ctx.loading
                        && let Some(key) = console::key_name(&event)
                        && let Some(requests) =
                            state.console.run_bound(&key, &mut state.ctx.settings)
                    {
                        for request in requests {
                            let console::Request::Command { name, args } = request else {
                                continue;
                            };
                            match state.run_command(&name, &args) {
                                Ok(false) => {}
                                Ok(true) => event_loop.exit(),
                                Err(e) => {
                                    self.error = Some(e);
                                    event_loop.exit();
                                }
                            }
                        }
                        return;
                    }
                    let was_down = state.input.is_down(code);
                    state.input.set_key(code, pressed);
                    if pressed && !was_down {
                        state.demo.key_pressed(&mut state.ctx, code);
                    }
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => state.modifiers = modifiers.state(),
            WindowEvent::MouseInput {
                button,
                state: pressed,
                ..
            } => {
                let down = pressed == ElementState::Pressed;
                // The game's menu (#352) takes the buttons: a left click takes the entry under
                // the cursor; the game gets none of them (their releases neither: none is held).
                if state.menu.is_open() {
                    if down
                        && button == MouseButton::Left
                        && let Some(m) = state.ctx.glyph_metrics()
                    {
                        let e = state.ctx.swapchain.extent();
                        let size = [e.width as f32, e.height as f32];
                        let action =
                            state
                                .menu
                                .click(state.cursor, size, m, &mut state.ctx.settings);
                        if state.menu_action(action) {
                            event_loop.exit();
                        }
                    }
                    return;
                }
                // A button bound to a console line (`bind mouse1 …`, D-069) runs it, as a key.
                if down
                    && !state.ctx.loading
                    && let Some(name) = input::button_name(button)
                    && let Some(requests) = state.console.run_bound(name, &mut state.ctx.settings)
                {
                    for request in requests {
                        let console::Request::Command { name, args } = request else {
                            continue;
                        };
                        match state.run_command(&name, &args) {
                            Ok(false) => {}
                            Ok(true) => event_loop.exit(),
                            Err(e) => {
                                self.error = Some(e);
                                event_loop.exit();
                            }
                        }
                    }
                    return;
                }
                state.input.set_button(button, down);
                // The right button turns the fly camera's view, unless the game's mouse look holds
                // the cursor and the button is the game's (aiming).
                if button == MouseButton::Right && !state.input.mouse_look {
                    state.input.looking = down;
                    let _ = state.ctx.window.set_cursor_grab(if down {
                        CursorGrabMode::Confined
                    } else {
                        CursorGrabMode::None
                    });
                    state.ctx.window.set_cursor_visible(!down);
                }
            }
            // As games do: out of the window (Alt+Tab, another window clicked) the cursor is the
            // system's again, free and shown, and no key, button or motion stays the game's; back
            // in it, the game's mouse look takes the cursor again, unless Esc let it go.
            WindowEvent::Focused(focused) => {
                state.input.unfocused = !focused;
                if !focused {
                    state.input.release_all();
                    state.input.looking = false;
                    state.input.mouse_delta = (0.0, 0.0);
                    free_cursor(&state.ctx.window);
                } else if state.input.mouse_look && !state.input.paused {
                    state.input.looking = true;
                    capture_cursor(&state.ctx.window);
                }
            }
            // Where the cursor stands: the menu's entry under it is chosen (#352).
            WindowEvent::CursorMoved { position, .. } => {
                state.cursor = [position.x as f32, position.y as f32];
                if state.menu.is_open()
                    && let Some(m) = state.ctx.glyph_metrics()
                {
                    let e = state.ctx.swapchain.extent();
                    let size = [e.width as f32, e.height as f32];
                    state.menu.hover(state.cursor, size, m, &state.ctx.settings);
                }
            }
            // The wheel's notches (a touchpad's pixels at 40 a notch): the fly camera's speed.
            WindowEvent::MouseWheel { delta, .. } => {
                state.input.wheel += match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        // The raw motion comes whichever window has the focus: the game's only while it has it.
        if let (Some(state), DeviceEvent::MouseMotion { delta }) = (self.state.as_mut(), event)
            && !state.input.unfocused
        {
            state.input.mouse_delta.0 += delta.0 as f32;
            state.input.mouse_delta.1 += delta.1 as f32;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match &self.state {
            // A hidden window is never asked to redraw: its frames are drawn from here until
            // the first one shows it.
            Some(state) if !state.shown => self.redraw(event_loop),
            Some(state) => state.ctx.window.request_redraw(),
            None => {}
        }
    }
}

impl<D: Demo> App<D> {
    /// One frame, then the window shown after its first (#219), and the exit at the frame
    /// limit.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if let Err(e) = state.frame() {
            self.error = Some(e);
            event_loop.exit();
            return;
        }
        if !state.shown {
            // Without activation when the window was made inactive (scripted runs).
            state.ctx.window.set_visible(true);
            state.shown = true;
        }
        if let Some(limit) = state.config.frame_limit
            && state.ctx.frames_rendered >= limit
        {
            event_loop.exit();
        }
    }
}

impl<D: Demo> Drop for State<D> {
    fn drop(&mut self) {
        // Resources are RAII; the GPU has to be done with them before the demo drops.
        self.ctx.device.wait_idle();
    }
}

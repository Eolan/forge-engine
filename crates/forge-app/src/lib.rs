//! Application shell for demos and tools: a window, input state, the frame loop on
//! `forge_gpu::Frames`, swapchain recreation, PNG capture and a fly camera.
//!
//! A demo implements [`Demo`] and calls [`run`], or [`run_loading`] when its start-up has heavy
//! CPU work to do behind a loading screen. The shell owns the GPU context and the
//! render graph of every frame: it imports the swapchain image, lets the demo declare its
//! passes, adds the overlay, the capture and the present transition, and executes the
//! graph, which derives every barrier.

#![forbid(unsafe_code)]

mod calibration;
mod camera;
mod content_light;
mod hdr;
mod input;
mod loading;
mod overlay;
mod profile;
mod ssaa;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use calibration::{Action, Calibration, Page, SettingsFile};
pub use camera::FlyCamera;
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
    MemoryLocation, RawImage, RenderGraph, ResourceState, ShaderCompiler, Surface, SurfaceMode,
    Swapchain, VENDOR_NVIDIA,
};
pub use hdr::{
    ContentLight, DisplayOutput, DisplaySettings, HdrMode, OFFSCREEN_FORMAT, PEAKS, preset_peak,
};
pub use input::Input;
pub use loading::Finish;
pub use overlay::{Canvas, Color, Overlay};
pub use profile::{MemorySample, OverlayMode, Profile};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
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
    /// Vertical sync.
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
    /// Whether the profiling overlay starts visible: `None` = yes when interactive, no when
    /// a frame limit is set (scripted captures); `Some` forces it. F1 toggles it at run time.
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
    /// it). On the display it needs the OS to show the display in HDR; F2 switches at run
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
    /// The frame profile behind the overlay; demos add counter lines to it.
    pub profile: Profile,
    /// The render graph's persistent side (transient heap, statistics).
    pub graph: RenderGraph,
    /// While a demo started with [`run_loading`] prepares on its thread (issue #25): the loading
    /// screen shows, and its frames do not count (`frames_rendered`, captures, the frame limit,
    /// the profile, the memory counters).
    pub loading: bool,
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
}

/// Runs a demo whose start-up has heavy CPU work (issue #25): `prepare` runs on a thread of its
/// own while the window shows a loading animation, and returns the step that finishes the demo
/// (uploads, pipelines), which runs on a worker of its own too while the animation goes on
/// (#201), with the [`Setup`] of the shell. The loading frames do not count: the demo's frames
/// number from 0, as with [`run`], so frame limits and captures are unchanged.
pub fn run_loading<D: Demo + Send>(
    config: AppConfig,
    prepare: impl FnOnce() -> Result<Finish<D>> + Send + 'static,
) -> Result<()> {
    run(config, move |ctx| loading::Stage::start(ctx, prepare))
}

/// Runs the demo built by `init` until the window closes, Escape is pressed or the frame
/// limit is reached.
pub fn run<D: Demo>(
    config: AppConfig,
    init: impl FnOnce(&mut Context) -> Result<D> + 'static,
) -> Result<()> {
    if !tracing::dispatcher::has_been_set() {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .init();
    }
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
        if let Some(light) = state.ctx.output.content_light {
            tracing::info!(
                "hdr: MaxCLL {:.1} nits, MaxFALL {:.1} nits",
                light.max_cll,
                light.max_fall
            );
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

/// `Digit1`..`Digit9` → 0..8 (the overlay's group fold keys).
fn digit_key(code: KeyCode) -> Option<usize> {
    Some(match code {
        KeyCode::Digit1 => 0,
        KeyCode::Digit2 => 1,
        KeyCode::Digit3 => 2,
        KeyCode::Digit4 => 3,
        KeyCode::Digit5 => 4,
        KeyCode::Digit6 => 5,
        KeyCode::Digit7 => 6,
        KeyCode::Digit8 => 7,
        KeyCode::Digit9 => 8,
        _ => return None,
    })
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
    /// The calibration pages (F5).
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

struct State<D: Demo> {
    demo: D,
    ctx: Context,
    input: Input,
    needs_resize: Option<PhysicalSize<u32>>,
    last_frame: Instant,
    last_title: Instant,
    config: AppConfig,
    /// `FORGE_WAIT_IDLE=1`: wait for the device after every submit (debugging).
    debug_wait_idle: bool,
    /// `FORGE_NO_TITLE=1`: never update the window title (debugging).
    debug_no_title: bool,
    /// `FORGE_STALL_MS=N`: sleep N ms after every frame (debugging).
    debug_stall_ms: u64,
    /// `FORGE_HDR_CYCLE=N`: press F2 every N frames (the HDR switch in scripted runs).
    debug_hdr_cycle: u64,
    /// The frame of the last scripted switch (a frame can start twice, after a resize).
    last_hdr_cycle: u64,
    overlay: Overlay,
    /// The preview, the content-light meter and the calibration pages.
    hdr: HdrPasses,
    /// The HDR mode asked for at start (`--hdr`, `FORGE_HDR`): the one F2 turns on.
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

    /// F2: HDR on or off. On, it is the mode asked for at start, else HDR10 when the OS shows
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

    /// F3: the next of ACES 2.0's peaks.
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

    /// F5: the calibration pages, from the first, in an HDR mode (#125).
    fn open_calibration(&mut self, page: Page) -> Result<()> {
        if !self.ctx.output.is_hdr() {
            tracing::warn!("the HDR calibration needs an HDR mode: F2 first");
            return Ok(());
        }
        self.hdr
            .calibration
            .open(&self.ctx.device, &self.ctx.shaders, &self.ctx.output, page)?;
        Ok(())
    }

    /// A key on the open calibration pages (`fine`: Shift is down).
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

    fn new(window: Arc<Window>, config: AppConfig, init: InitFn<D>) -> Result<Self> {
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
            config.vsync,
        )?;
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
            .then(|| SettingsFile::new(root.join("settings/display.txt")));
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
        let shaders = ShaderCompiler::new(
            root.join("shaders"),
            forge_core::derived::cache_dir(forge_core::derived::CacheKind::Shaders),
            config.optimize_shaders,
        )?;
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
            _ if config.overlay.unwrap_or(config.frame_limit.is_none()) => OverlayMode::Compact,
            _ => OverlayMode::Off,
        };
        let graph = RenderGraph::new(&device);
        let mut ctx = Context {
            device,
            swapchain,
            output,
            frames,
            shaders,
            window,
            frames_rendered: 0,
            profile: Profile::new(overlay_mode),
            graph,
            loading: false,
            ssaa: 1,
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
        let demo = init(&mut ctx)?;
        Ok(Self {
            demo,
            ctx,
            hdr,
            hdr_requested: requested,
            settings_file,
            monitor,
            input: Input::default(),
            needs_resize: None,
            last_frame: Instant::now(),
            last_title: Instant::now(),
            config,
            debug_wait_idle: std::env::var_os("FORGE_WAIT_IDLE").is_some_and(|v| v != "0"),
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
        if let Some(size) = self.needs_resize.take() {
            if size.width == 0 || size.height == 0 {
                self.needs_resize = Some(size);
                return Ok(());
            }
            let current = self.ctx.swapchain.extent();
            if size.width != current.width || size.height != current.height {
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
        let was_loading = self.ctx.loading;
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
        if self.ctx.loading {
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
            return Ok(());
        };
        self.ctx
            .profile
            .cpu_zone("cpu/acquire swapchain image", ms_since(acquire_start));
        let extent = self.ctx.swapchain.extent();
        let frame_number = self.ctx.frames_rendered;
        let capture_path = match &self.config.capture {
            _ if self.ctx.loading => None,
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

        // The window's format changes with HDR (F2): supersampling's pass follows it.
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
            if calibrating || (self.ctx.profile.is_visible() && !self.ctx.loading) {
                if !calibrating && self.ctx.output.is_hdr() {
                    self.ctx.profile.counter(self.ctx.output.describe());
                }
                let title = self.config.title.clone();
                let canvas = self.overlay.begin(extent);
                if calibrating {
                    self.hdr.calibration.layout(canvas, &self.ctx.output);
                } else {
                    self.ctx.profile.layout(canvas, &title, extent);
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
        if !self.ctx.loading {
            self.ctx.frames_rendered += 1;
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
    state: Option<State<D>>,
    error: Option<anyhow::Error>,
}

impl<D: Demo> ApplicationHandler for App<D> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let mut attributes = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(PhysicalSize::new(self.config.width, self.config.height));
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
        if self.config.frame_limit.is_some() {
            attributes = attributes.with_active(false);
        }
        let Some(init) = self.init.take() else {
            return;
        };
        match event_loop.create_window(attributes) {
            Ok(window) => match State::new(Arc::new(window), self.config.clone(), init) {
                Ok(state) => self.state = Some(state),
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
                    if pressed && code == KeyCode::Escape {
                        event_loop.exit();
                        return;
                    }
                    if pressed && code == KeyCode::F1 {
                        state.ctx.profile.cycle_mode();
                        return;
                    }
                    // The HDR output (issue #94): on and off, the peak, the preview's false
                    // colours.
                    if pressed && code == KeyCode::F2 {
                        if let Err(e) = state.toggle_hdr() {
                            self.error = Some(e);
                            event_loop.exit();
                        }
                        return;
                    }
                    if pressed && code == KeyCode::F3 {
                        state.next_peak();
                        return;
                    }
                    if pressed && code == KeyCode::F4 {
                        if let Some(preview) = &mut state.hdr.preview {
                            preview.false_colours = !preview.false_colours;
                        }
                        return;
                    }
                    if pressed && code == KeyCode::F5 {
                        if let Err(e) = state.open_calibration(Page::Peak) {
                            self.error = Some(e);
                            event_loop.exit();
                        }
                        return;
                    }
                    if pressed
                        && state.ctx.profile.is_visible()
                        && let Some(group) = digit_key(code)
                    {
                        state.ctx.profile.toggle_group(group);
                        return;
                    }
                    let was_down = state.input.is_down(code);
                    state.input.set_key(code, pressed);
                    if pressed && !was_down {
                        state.demo.key_pressed(&mut state.ctx, code);
                    }
                }
            }
            WindowEvent::MouseInput {
                button: MouseButton::Right,
                state: pressed,
                ..
            } => {
                let looking = pressed == ElementState::Pressed;
                state.input.looking = looking;
                let _ = state.ctx.window.set_cursor_grab(if looking {
                    CursorGrabMode::Confined
                } else {
                    CursorGrabMode::None
                });
                state.ctx.window.set_cursor_visible(!looking);
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = state.frame() {
                    self.error = Some(e);
                    event_loop.exit();
                    return;
                }
                if let Some(limit) = state.config.frame_limit
                    && state.ctx.frames_rendered >= limit
                {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (Some(state), DeviceEvent::MouseMotion { delta }) = (self.state.as_mut(), event) {
            state.input.mouse_delta.0 += delta.0 as f32;
            state.input.mouse_delta.1 += delta.1 as f32;
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.ctx.window.request_redraw();
        }
    }
}

impl<D: Demo> Drop for State<D> {
    fn drop(&mut self) {
        // Resources are RAII; the GPU has to be done with them before the demo drops.
        self.ctx.device.wait_idle();
    }
}

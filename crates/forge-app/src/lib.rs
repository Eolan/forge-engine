//! Application shell for demos and tools: a window, input state, the frame loop on
//! `forge_gpu::Frames`, swapchain recreation, PNG capture and a fly camera.
//!
//! A demo implements [`Demo`] and calls [`run`]. The shell owns the GPU context and the
//! render graph of every frame: it imports the swapchain image, lets the demo declare its
//! passes, adds the overlay, the capture and the present transition, and executes the
//! graph, which derives every barrier.

#![forbid(unsafe_code)]

mod camera;
mod input;
mod overlay;
mod profile;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
pub use camera::FlyCamera;
/// Re-exported for demos that declare their own per-frame targets.
pub use forge_gpu::TransientDesc;
/// Re-exported so demos can name Vulkan types without depending on `forge-gpu` directly.
pub use forge_gpu::vk;
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, Commands, Device, DeviceOptions, FrameGraph, FrameSlot,
    Frames, GraphBuffer, GraphStats, ImageAccess, ImageHandle, Instance, MemoryCategory,
    MemoryLocation, RawImage, RenderGraph, ResourceState, ShaderCompiler, Surface, Swapchain,
};
pub use input::Input;
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
        }
    }
}

/// Everything a demo needs from the shell.
pub struct Context {
    /// The device.
    pub device: Arc<Device>,
    /// The swapchain (recreated on resize; watch [`Context::extent`]).
    pub swapchain: Swapchain,
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
    _surface: Arc<Surface>,
    _instance: Arc<Instance>,
}

impl Context {
    /// Current swapchain size.
    pub fn extent(&self) -> vk::Extent2D {
        self.swapchain.extent()
    }

    /// Aspect ratio of the swapchain.
    pub fn aspect(&self) -> f32 {
        let e = self.extent();
        e.width as f32 / e.height.max(1) as f32
    }
}

/// One frame handed to [`Demo::render`].
pub struct FrameInfo<'f> {
    /// The frame's render graph: the demo declares its passes into it.
    pub graph: FrameGraph<'f>,
    /// The swapchain image (contents undefined on entry). The demo's last pass on it must
    /// write it; the shell adds the overlay, the capture and the present transition after.
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

struct State<D: Demo> {
    demo: D,
    ctx: Context,
    input: Input,
    needs_resize: Option<PhysicalSize<u32>>,
    last_frame: Instant,
    last_title: Instant,
    config: AppConfig,
    /// `FORGE_FRAME_BARRIER=1`: a full memory barrier at the start of every frame (debugging).
    debug_frame_barrier: bool,
    /// `FORGE_WAIT_IDLE=1`: wait for the device after every submit (debugging).
    debug_wait_idle: bool,
    /// `FORGE_NO_TITLE=1`: never update the window title (debugging).
    debug_no_title: bool,
    /// `FORGE_STALL_MS=N`: sleep N ms after every frame (debugging).
    debug_stall_ms: u64,
    overlay: Overlay,
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

    fn new(window: Arc<Window>, config: AppConfig, init: InitFn<D>) -> Result<Self> {
        let display = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();
        let validation = config.validate || cfg!(debug_assertions);
        let root = workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let instance = if config.streamline {
            let sdk = std::env::var_os("FORGE_STREAMLINE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("streamline-sdk/bin/x64"));
            match Instance::with_streamline(c"forge", validation, Some(display), &sdk) {
                Ok(instance) => instance,
                Err(error) => {
                    tracing::warn!(%error, sdk = %sdk.display(), "no Streamline: DLSS unavailable");
                    Instance::new(c"forge", validation, Some(display))?
                }
            }
        } else {
            Instance::new(c"forge", validation, Some(display))?
        };
        let instance = Arc::new(instance);
        let surface = instance.create_surface(display, window_handle)?;
        let device = Device::with_options(
            Arc::clone(&instance),
            Some(surface.raw()),
            DeviceOptions {
                no_mesh_shader: config.force_fallback,
            },
        )?;
        let size = window.inner_size();
        let swapchain = Swapchain::new(
            Arc::clone(&device),
            Arc::clone(&surface),
            size.width,
            size.height,
            config.vsync,
        )?;
        let frames = Frames::new(Arc::clone(&device), swapchain.image_count())?;
        let shaders = ShaderCompiler::new(
            root.join("shaders"),
            root.join("shader-cache"),
            config.optimize_shaders,
        )?;
        let font_path = std::env::var_os("FORGE_OVERLAY_FONT")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf"));
        let font_px = std::env::var("FORGE_OVERLAY_FONT_PX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(14.0);
        let overlay = Overlay::new(
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
            frames,
            shaders,
            window,
            frames_rendered: 0,
            profile: Profile::new(overlay_mode),
            graph,
            _surface: surface,
            _instance: instance,
        };
        let demo = init(&mut ctx)?;
        Ok(Self {
            demo,
            ctx,
            input: Input::default(),
            needs_resize: None,
            last_frame: Instant::now(),
            last_title: Instant::now(),
            config,
            debug_frame_barrier: std::env::var_os("FORGE_FRAME_BARRIER").is_some_and(|v| v != "0"),
            debug_wait_idle: std::env::var_os("FORGE_WAIT_IDLE").is_some_and(|v| v != "0"),
            debug_no_title: std::env::var_os("FORGE_NO_TITLE").is_some_and(|v| v != "0"),
            debug_stall_ms: std::env::var("FORGE_STALL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
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
            "graph: {} passes, {} image + {} memory barriers; transients {} images, {:.1} MB in a {:.1} MB heap{}; {} rebuilds, {} retired",
            g.passes,
            g.image_barriers,
            g.memory_barriers,
            g.transient_images,
            g.transient_bytes as f64 / 1e6,
            g.heap_bytes as f64 / 1e6,
            if g.aliased { " (aliased)" } else { "" },
            g.heap_rebuilds,
            g.pending_destructions
        ));
        let update_start = Instant::now();
        {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("update");
            self.demo.update(&mut self.ctx, &self.input, dt);
        }
        self.ctx
            .profile
            .cpu_zone("cpu/update", ms_since(update_start));
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
        self.ctx.profile.gpu_zones(self.ctx.frames.gpu_zones());
        #[cfg(feature = "profiling")]
        self.tracy_gpu_zones();
        #[cfg(feature = "profiling")]
        if let Some(ms) = slot.previous_gpu_ms {
            tracy_client::plot!("gpu ms", ms);
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
        if capture_path.is_some()
            || self
                .memory_mark
                .is_none_or(|mark| mark.at.elapsed() >= MEMORY_SAMPLE_PERIOD)
        {
            self.sample_memory_since(self.memory_mark);
        }
        let capture = capture_path
            .map(|path| {
                let buffer = self.ctx.device.create_buffer(BufferDesc {
                    size: u64::from(extent.width) * u64::from(extent.height) * 4,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: "capture",
                })?;
                Ok::<_, forge_gpu::GpuError>((path, GraphBuffer::new(buffer)))
            })
            .transpose()?;

        let cb = self.ctx.frames.begin(slot)?;
        let device = Arc::clone(&self.ctx.device);
        let timer_slot = self.ctx.frames.timer_slot(slot);
        let record_start = Instant::now();
        {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("record");
            let mut graph = FrameGraph::new(extent);
            // The acquired swapchain image: contents undefined, usable once the acquire
            // semaphore's stage (colour output) has passed.
            let target = graph.import_raw(RawImage {
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
                name: "swapchain",
            });
            let mut frame = FrameInfo {
                graph,
                target,
                slot,
                image_index,
                dt,
            };
            self.demo.render(&mut self.ctx, &mut frame)?;
            if self.ctx.profile.is_visible() {
                let title = self.config.title.clone();
                let canvas = self.overlay.begin(extent);
                self.ctx.profile.layout(canvas, &title, extent);
                self.overlay
                    .draw(&mut frame.graph, slot.index, target, extent);
            }
            if let Some((_, buffer)) = &capture {
                // The copy, then the host read after the wait below: declared, so the
                // device's writes are made visible to the host.
                let handle = frame.graph.import_buffer(buffer);
                frame
                    .graph
                    .pass("app/capture")
                    .image(target, ImageAccess::TransferSrc)
                    .buffer(handle, BufferAccess::TransferDst)
                    .run(move |resources, commands| {
                        commands.copy_image_to_buffer(resources.image(target).raw, extent, buffer);
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
                .image(target, ImageAccess::Present)
                .run(|_, _| Ok(()));
            let commands = Commands::new(&device, cb).with_timers(&timer_slot);
            if self.debug_frame_barrier {
                // Debugging aid (`FORGE_FRAME_BARRIER=1`): serialise frames on the GPU.
                commands.memory_barrier(
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
                    vk::PipelineStageFlags2::ALL_COMMANDS,
                    vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE,
                );
            }
            self.graph_stats =
                self.ctx
                    .graph
                    .execute(frame.graph, &commands, &mut self.ctx.frames)?;
        }
        self.ctx
            .profile
            .cpu_zone("cpu/record commands", ms_since(record_start));
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
        if let Some((path, buffer)) = capture {
            self.ctx.device.wait_idle();
            save_capture(&path, &buffer, extent, self.ctx.swapchain.format())?;
            tracing::info!(path = %path.display(), "captured frame {}", self.ctx.frames_rendered);
        }
        self.ctx.frames_rendered += 1;
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
    let mut pixels = vec![0_u8; (extent.width * extent.height * 4) as usize];
    buffer.read(0, &mut pixels);
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
                    if pressed && code == KeyCode::Escape {
                        event_loop.exit();
                        return;
                    }
                    if pressed && code == KeyCode::F1 {
                        state.ctx.profile.cycle_mode();
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

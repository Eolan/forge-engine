//! Application shell for demos and tools: a window, input state, the frame loop on
//! `forge_gpu::Frames`, swapchain recreation, PNG capture and a fly camera.
//!
//! A demo implements [`Demo`] and calls [`run`]. The shell owns the GPU context and the
//! swapchain image transitions (undefined → colour attachment before `render`, colour
//! attachment → present after); the demo records everything in between.

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
/// Re-exported so demos can record barriers without depending on `forge-gpu` directly.
pub use forge_gpu::vk;
use forge_gpu::{
    Buffer, BufferDesc, Commands, Device, FrameSlot, Frames, Instance, MemoryLocation,
    ShaderCompiler, Surface, Swapchain,
};
pub use input::Input;
pub use overlay::{Canvas, Color, Overlay};
pub use profile::{OverlayMode, Profile};
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
pub struct FrameInfo<'a> {
    /// Recording commands for this frame.
    pub commands: Commands<'a>,
    /// The frame slot (index, number, previous GPU time).
    pub slot: FrameSlot,
    /// Swapchain image being rendered, already in `COLOR_ATTACHMENT_OPTIMAL`.
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

    /// Records the frame. The swapchain image is a colour attachment on entry and must be
    /// left in `COLOR_ATTACHMENT_OPTIMAL`.
    fn render(&mut self, ctx: &mut Context, frame: &FrameInfo<'_>) -> Result<()>;

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
    if let Some(state) = app.state.take() {
        let frames = state.ctx.frames_rendered;
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
    #[cfg(feature = "profiling")]
    tracy_gpu: Option<tracy_client::GpuContext>,
}

type InitFn<D> = Box<dyn FnOnce(&mut Context) -> Result<D>>;

impl<D: Demo> State<D> {
    fn new(window: Arc<Window>, config: AppConfig, init: InitFn<D>) -> Result<Self> {
        let display = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();
        let validation = config.validate || cfg!(debug_assertions);
        let instance = Arc::new(Instance::new(c"forge", validation, Some(display))?);
        let surface = instance.create_surface(display, window_handle)?;
        let device = Device::new(Arc::clone(&instance), Some(surface.raw()))?;
        let size = window.inner_size();
        let swapchain = Swapchain::new(
            Arc::clone(&device),
            Arc::clone(&surface),
            size.width,
            size.height,
            config.vsync,
        )?;
        let frames = Frames::new(Arc::clone(&device), swapchain.image_count())?;
        let root = workspace_root_from(env!("CARGO_MANIFEST_DIR"));
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
        let mut ctx = Context {
            device,
            swapchain,
            frames,
            shaders,
            window,
            frames_rendered: 0,
            profile: Profile::new(overlay_mode),
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
        let capture = capture_path
            .map(|path| {
                let buffer = self.ctx.device.create_buffer(BufferDesc {
                    size: u64::from(extent.width) * u64::from(extent.height) * 4,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    name: "capture",
                })?;
                Ok::<_, forge_gpu::GpuError>((path, buffer))
            })
            .transpose()?;

        let cb = self.ctx.frames.begin(slot)?;
        let color_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let swap_image = self.ctx.swapchain.image(image_index);
        let barrier = || {
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(swap_image)
                .subresource_range(color_range)
        };
        let device = Arc::clone(&self.ctx.device);
        let timer_slot = self.ctx.frames.timer_slot(slot);
        let record_start = Instant::now();
        {
            #[cfg(feature = "profiling")]
            let _zone = tracy_client::span!("record");
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
            commands.image_barriers(&[barrier()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)]);
            let frame = FrameInfo {
                commands,
                slot,
                image_index,
                dt,
            };
            self.demo.render(&mut self.ctx, &frame)?;
            let commands = frame.commands;
            commands.mark("app/unmarked demo work");
            if self.ctx.profile.is_visible() {
                let title = self.config.title.clone();
                let canvas = self.overlay.begin(extent);
                self.ctx.profile.layout(canvas, &title, extent);
                self.overlay.draw(
                    &commands,
                    slot.index,
                    self.ctx.swapchain.view(image_index),
                    extent,
                );
                commands.mark("app/overlay");
            }
            if let Some((_, buffer)) = &capture {
                commands.image_barriers(&[barrier()
                    .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)]);
                commands.copy_image_to_buffer(swap_image, extent, buffer);
                commands.image_barriers(&[barrier()
                    .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
                    .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)]);
            } else {
                commands.image_barriers(&[barrier()
                    .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                    .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
                    .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)]);
            }
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

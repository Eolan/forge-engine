//! NVIDIA's NRC (Neural Radiance Cache, v0.16), D-088's trial under the diffuse bounce: a small
//! network trained every frame on the light of sparse paths, asked for the light leaving the
//! points where the bounce's rays stop (`docs/research/sharc-and-nrc.md`).
//!
//! NRC is a binary release under the NVIDIA RTX SDKs licence, never in the repository:
//! `tools/fetch-nrc.sh` builds Forge's C shim over its C++ API (`tools/forge_nrc.cpp`) into
//! the git-ignored `nrc-sdk/bin`, beside `NRC_Vulkan.dll` and CUDA 13's runtime, and [`Nrc::load`]
//! loads it at run time. Built without the `nrc` feature, or with no library there, it fails and
//! the bounce keeps its other caches.
//!
//! NRC runs its CUDA kernels inside Vulkan's command buffers (`VK_NVX_binary_import`), so it
//! needs no interop: the device is created with its extensions ([`crate::DeviceOptions::nrc`]),
//! and every call records into the frame's command buffer from inside a render-graph pass, which
//! declares the buffers it touches. NRC allocates its buffers itself (several sit on CUDA's
//! shared heap); [`Nrc::buffer`] hands them out wrapped as graph buffers, which the graph tracks
//! as Forge's own and never frees. Each frame:
//! 1. [`Nrc::end_frame`] for the frame before (submitted by then), then [`Nrc::begin_frame`]
//!    (its counters cleared) and the shaders' `NrcConstants` it returns;
//! 2. Forge's passes write the queries (the bounce's hits) and the training paths;
//! 3. [`Nrc::query_and_train`]: the network's answers at the queries, its training on the paths;
//! 4. Forge's resolve reads the answers.
//!
//! The structures below mirror the shim's (`ForgeNrcContext`, `ForgeNrcFrame`, `ForgeNrcBuffer`):
//! their sizes are checked on both sides, and the shim's NRC version when it loads.

use std::path::Path;
use std::sync::Arc;

use ash::vk;
use bytemuck::{Pod, Zeroable};

use crate::Device;
use crate::commands::Commands;
use crate::error::Result;
use crate::graph::GraphBuffer;

/// The NRC release (major, minor) the shim must have been built against.
pub const NRC_VERSION: (u32, u32) = (0, 16);
/// The shim's file in the library's folder.
pub const NRC_SHIM: &str = "forge_nrc.dll";
/// The size of NRC's shader constants (`NrcConstants`), which [`Nrc::begin_frame`] returns.
pub const CONSTANTS_BYTES: usize = 96;

/// NRC's buffers, in its order (`nrc::BufferIdx`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NrcBuffer {
    /// The two counters: queries and training records.
    Counter,
    /// Per pixel: its query's index and the light's weight before it.
    QueryPathInfo,
    /// Per training path.
    TrainingPathInfo,
    /// Per training path's vertex.
    TrainingPathVertices,
    /// NRC's own: the training targets.
    TrainingRadiance,
    /// NRC's own: the training records' inputs.
    TrainingRadianceParams,
    /// The network's answer per query.
    QueryRadiance,
    /// The queries' inputs.
    QueryRadianceParams,
    /// The debug views' (none without NRC's debug buffers).
    DebugTrainingPathInfo,
}

impl NrcBuffer {
    /// Every buffer, in NRC's order.
    pub const ALL: [Self; 9] = [
        Self::Counter,
        Self::QueryPathInfo,
        Self::TrainingPathInfo,
        Self::TrainingPathVertices,
        Self::TrainingRadiance,
        Self::TrainingRadianceParams,
        Self::QueryRadiance,
        Self::QueryRadianceParams,
        Self::DebugTrainingPathInfo,
    ];
}

/// The cache's settings (`nrc::ContextSettings`): a change reconfigures it ([`Nrc::configure`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct NrcSettings {
    /// 1: the network learns the light reaching a point (its albedo divided out), not leaving it.
    pub learn_irradiance: u32,
    /// 1: the cache holds the points' direct light too.
    pub include_direct: u32,
    /// 1: start the cache over.
    pub request_reset: u32,
    /// Query paths per pixel.
    pub samples_per_pixel: u32,
    /// The scene's bounds: every point a ray can meet is inside them.
    pub bounds_min: [f32; 3],
    /// The smallest detail the cache resolves, metres (NRC's default 0.01).
    pub feature_size: f32,
    /// The scene's bounds' other corner.
    pub bounds_max: [f32; 3],
    /// A training path's most vertices.
    pub max_path_vertices: u32,
    /// The frame's size: one query path per pixel.
    pub frame: [u32; 2],
    /// The training paths' size ([`Nrc::training_size`]).
    pub training: [u32; 2],
}

/// One frame's settings (`nrc::FrameSettings`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct NrcFrameSettings {
    /// About the mean light of a bright scene, in the light's units: NRC scales by it.
    pub max_average_radiance: f32,
    /// The termination heuristic's threshold for the queries (NRC's 0.1).
    pub termination: f32,
    /// The same for the training paths.
    pub training_termination: f32,
    /// The optimiser's learning rate (NRC's 0.01).
    pub learning_rate: f32,
    /// The training paths traced this frame, at most [`NrcSettings::training`] (0, 0: all).
    pub used_training: [u32; 2],
    /// Training batches a frame (NRC's 4; 16 K records each).
    pub training_iterations: u32,
    /// 0 freezes the cache.
    pub train: u32,
    /// 1: no query at a mirror, the path goes on.
    pub skip_delta_vertices: u32,
    /// The share of the training paths traced to their full length (NRC's 1/16).
    pub proportion_unbiased: f32,
    /// The weight of the cache's own answers at the training paths' ends (NRC's 1).
    pub self_training_attenuation: f32,
    /// NRC's resolve mode (0: its default), which only its own resolve reads.
    pub resolve_mode: u32,
    /// The share of the paths' first vertices whose records train the cache (NRC's 0.05, for a
    /// camera's primary hits; Forge's paths start at the bounce's hits, where the queries are).
    pub proportion_primary: f32,
}

impl Default for NrcFrameSettings {
    /// NRC's own defaults (`nrc::FrameSettings`).
    fn default() -> Self {
        Self {
            max_average_radiance: 1.0,
            termination: 0.1,
            training_termination: 0.1,
            learning_rate: 1e-2,
            used_training: [0, 0],
            training_iterations: 4,
            train: 1,
            skip_delta_vertices: 0,
            proportion_unbiased: 0.0625,
            self_training_attenuation: 1.0,
            resolve_mode: 0,
            proportion_primary: 0.05,
        }
    }
}

const _: () = assert!(std::mem::size_of::<NrcSettings>() == 64);
const _: () = assert!(std::mem::size_of::<NrcFrameSettings>() == 52);

/// NRC on this device: the shim, its context and the buffers of its last configuration.
pub struct Nrc(imp::Nrc);

impl Nrc {
    /// Loads [`NRC_SHIM`] from `dir` (with `NRC_Vulkan.dll` and CUDA's runtime beside it), checks
    /// NRC's signature and version and creates its context. Fails without the `nrc` feature, on a
    /// device created without [`crate::DeviceOptions::nrc`] or without NRC's needs.
    pub fn load(device: &Arc<Device>, dir: &Path) -> Result<Self> {
        imp::Nrc::load(device, dir).map(Self)
    }

    /// NRC's training paths' size for frames of `frame` and `iterations` training batches a frame
    /// (`nrc::ComputeIdealTrainingDimensions`).
    pub fn training_size(&self, frame: vk::Extent2D, iterations: u32) -> vk::Extent2D {
        self.0.training_size(frame, iterations)
    }

    /// (Re)configures the cache for `settings` after waiting for the device to be idle: NRC frees
    /// and makes its buffers again, so the graph buffers [`Nrc::buffer`] gave are dropped.
    pub fn configure(&mut self, settings: &NrcSettings) -> Result<()> {
        self.0.configure(settings)
    }

    /// The settings of the last [`Nrc::configure`], if any.
    pub fn settings(&self) -> Option<&NrcSettings> {
        self.0.settings()
    }

    /// One of NRC's buffers since the last [`Nrc::configure`] (`None` before, or when NRC made
    /// none: the debug one).
    pub fn buffer(&self, which: NrcBuffer) -> Option<&GraphBuffer> {
        self.0.buffer(which)
    }

    /// Records NRC's start of a frame (its counters cleared) into `commands`, from inside a graph
    /// pass that writes [`NrcBuffer::Counter`], and returns the shaders' `NrcConstants`.
    pub fn begin_frame(
        &self,
        commands: &Commands<'_>,
        frame: &NrcFrameSettings,
    ) -> Result<[u8; CONSTANTS_BYTES]> {
        self.0.begin_frame(commands, frame)
    }

    /// Records the network's answers at this frame's queries and its training on this frame's
    /// paths into `commands`, from inside a graph pass that reads and writes every buffer.
    pub fn query_and_train(&self, commands: &Commands<'_>) -> Result<()> {
        self.0.query_and_train(commands)
    }

    /// NRC's end of the last frame [`Nrc::begin_frame`] started, once its command buffer was
    /// submitted to the graphics queue; nothing when no frame is open.
    pub fn end_frame(&self) -> Result<()> {
        self.0.end_frame()
    }
}

#[cfg(all(feature = "nrc", windows))]
mod imp {
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ash::vk;
    use ash::vk::Handle as _;
    use parking_lot::Mutex;

    use super::{CONSTANTS_BYTES, NRC_SHIM, NRC_VERSION, NrcBuffer, NrcFrameSettings, NrcSettings};
    use crate::commands::Commands;
    use crate::error::{GpuError, Result};
    use crate::graph::GraphBuffer;
    use crate::memory::Buffer;
    use crate::{Device, FRAMES_IN_FLIGHT, MemoryCategory};

    /// `ForgeNrcBuffer`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct RawBuffer {
        buffer: u64,
        size: u64,
        offset: u64,
        address: u64,
    }

    const _: () = assert!(std::mem::size_of::<RawBuffer>() == 32);

    type Log = unsafe extern "C" fn(*const c_char, c_int);
    type Version = unsafe extern "C" fn() -> u32;
    type ConstantsSize = unsafe extern "C" fn() -> u32;
    type TrainingSize = unsafe extern "C" fn(u32, u32, u32, *mut u32);
    type Create = unsafe extern "C" fn(
        vk::Instance,
        vk::PhysicalDevice,
        vk::Device,
        *const u16,
        u32,
        Option<Log>,
        *mut *mut c_void,
    ) -> c_int;
    type Destroy = unsafe extern "C" fn(*mut c_void);
    type Configure =
        unsafe extern "C" fn(*mut c_void, *const NrcSettings, *mut RawBuffer, u32) -> c_int;
    type BeginFrame = unsafe extern "C" fn(
        *mut c_void,
        vk::CommandBuffer,
        *const NrcFrameSettings,
        *mut c_void,
    ) -> c_int;
    type QueryAndTrain = unsafe extern "C" fn(*mut c_void, vk::CommandBuffer) -> c_int;
    type EndFrame = unsafe extern "C" fn(*mut c_void, vk::Queue) -> c_int;

    /// NRC's log lines, into Forge's.
    unsafe extern "C" fn log(message: *const c_char, level: c_int) {
        if message.is_null() {
            return;
        }
        // SAFETY: NRC passes a NUL-terminated string that lives for the call.
        let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        let text = text.trim();
        match level {
            4 => tracing::error!(target: "nrc", "{text}"),
            3 => tracing::warn!(target: "nrc", "{text}"),
            2 => tracing::info!(target: "nrc", "{text}"),
            _ => tracing::debug!(target: "nrc", "{text}"),
        }
    }

    /// The shim and the context it made.
    struct Library {
        context: *mut c_void,
        destroy: Destroy,
        training_size: TrainingSize,
        configure: Configure,
        begin_frame: BeginFrame,
        query_and_train: QueryAndTrain,
        end_frame: EndFrame,
        // Last: the functions above point into it.
        _library: libloading::Library,
    }

    // SAFETY: NRC's context has no thread affinity; Forge calls it under `Nrc::library`'s mutex.
    unsafe impl Send for Library {}

    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: the context was created by this library and is destroyed once, the device
            // idle (`Nrc`'s drop waits for it).
            unsafe { (self.destroy)(self.context) };
        }
    }

    pub(in crate::nrc) struct Nrc {
        // Dropped in this order: the buffers while NRC still owns their memory, then NRC's
        // context, then (maybe the last) device.
        buffers: Vec<Option<GraphBuffer>>,
        library: Mutex<Library>,
        settings: Option<NrcSettings>,
        /// A frame was begun and not ended.
        open: AtomicBool,
        device: Arc<Device>,
    }

    fn status(call: &str, code: c_int) -> Result<()> {
        if code == 0 {
            return Ok(());
        }
        // `nrc::Status`, and the shim's 100 and 101.
        let name = match code {
            1 => "SDK version mismatch",
            2 => "already initialized",
            3 => "not initialized",
            4 => "internal error",
            5 => "memory not provided",
            6 => "out of memory",
            7 => "allocation failed",
            8 => "bad JSON",
            9 => "wrong parameter",
            10 => "unsupported driver",
            11 => "unsupported hardware",
            100 => "NRC_Vulkan.dll's signature does not verify",
            101 => "NRC_Vulkan.dll does not load",
            _ => "unknown status",
        };
        Err(GpuError::Nrc(format!("{call}: {name} ({code})")))
    }

    impl Nrc {
        pub(in crate::nrc) fn load(device: &Arc<Device>, dir: &Path) -> Result<Self> {
            if !device.features().nrc {
                return Err(GpuError::Nrc(
                    "the device was made without NRC's extensions (an NVIDIA GPU, \
                     VK_NVX_binary_import, VK_NVX_image_view_handle)"
                        .into(),
                ));
            }
            // Backslashes only: the shim loads NRC by this path with
            // `LOAD_WITH_ALTERED_SEARCH_PATH`, which a forward slash turns off.
            let dir: std::path::PathBuf = dir.components().collect();
            let path = dir.join(NRC_SHIM);
            if !path.exists() {
                return Err(GpuError::Nrc(format!(
                    "{} not found (tools/fetch-nrc.sh builds it)",
                    path.display()
                )));
            }
            // SAFETY: loading the shim runs no initialisation code beyond its C++ runtime's:
            // NRC_Vulkan.dll is delay-loaded, by `forge_nrc_create` once its signature checks.
            let library = unsafe {
                libloading::os::windows::Library::load_with_flags(
                    &path,
                    libloading::os::windows::LOAD_WITH_ALTERED_SEARCH_PATH,
                )
            }
            .map(libloading::Library::from)
            .map_err(|error| GpuError::Nrc(format!("cannot load {}: {error}", path.display())))?;
            macro_rules! function {
                ($name:literal, $kind:ty) => {{
                    // SAFETY: the symbol is one of the shim's exported C functions
                    // (`tools/forge_nrc.cpp`), declared with its signature.
                    let symbol = unsafe { library.get::<$kind>($name) }.map_err(|error| {
                        GpuError::Nrc(format!("{}: {error}", String::from_utf8_lossy($name)))
                    })?;
                    *symbol
                }};
            }
            let version = function!(b"forge_nrc_version\0", Version);
            let constants_size = function!(b"forge_nrc_constants_size\0", ConstantsSize);
            let create = function!(b"forge_nrc_create\0", Create);
            // SAFETY: plain getters.
            let (version, constants) = unsafe { (version(), constants_size()) };
            if (version >> 16, version & 0xFFFF) != NRC_VERSION {
                return Err(GpuError::Nrc(format!(
                    "the shim was built against NRC {}.{}, Forge needs {}.{} (tools/fetch-nrc.sh)",
                    version >> 16,
                    version & 0xFFFF,
                    NRC_VERSION.0,
                    NRC_VERSION.1
                )));
            }
            if constants as usize != CONSTANTS_BYTES {
                return Err(GpuError::Nrc(format!(
                    "NrcConstants is {constants} bytes, Forge expects {CONSTANTS_BYTES}"
                )));
            }
            let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
            let mut context = std::ptr::null_mut();
            // SAFETY: the handles are this device's, alive while the context is; `wide` is
            // NUL-terminated and outlives the call (the shim keeps its own copy, which NRC points
            // at); `log` lives for the process.
            let code = unsafe {
                create(
                    device.instance().raw().handle(),
                    device.physical(),
                    device.raw().handle(),
                    wide.as_ptr(),
                    FRAMES_IN_FLIGHT as u32,
                    Some(log),
                    &mut context,
                )
            };
            status("forge_nrc_create", code)?;
            let library = Library {
                context,
                destroy: function!(b"forge_nrc_destroy\0", Destroy),
                training_size: function!(b"forge_nrc_training_size\0", TrainingSize),
                configure: function!(b"forge_nrc_configure\0", Configure),
                begin_frame: function!(b"forge_nrc_begin_frame\0", BeginFrame),
                query_and_train: function!(b"forge_nrc_query_and_train\0", QueryAndTrain),
                end_frame: function!(b"forge_nrc_end_frame\0", EndFrame),
                _library: library,
            };
            tracing::info!(dir = %dir.display(), "NRC {}.{} loaded", NRC_VERSION.0, NRC_VERSION.1);
            Ok(Self {
                buffers: Vec::new(),
                library: Mutex::new(library),
                settings: None,
                open: AtomicBool::new(false),
                device: Arc::clone(device),
            })
        }

        pub(in crate::nrc) fn training_size(
            &self,
            frame: vk::Extent2D,
            iterations: u32,
        ) -> vk::Extent2D {
            let library = self.library.lock();
            let mut out = [0u32; 2];
            // SAFETY: a pure function of its arguments; `out` holds two values.
            unsafe {
                (library.training_size)(frame.width, frame.height, iterations, out.as_mut_ptr())
            };
            vk::Extent2D {
                width: out[0],
                height: out[1],
            }
        }

        pub(in crate::nrc) fn configure(&mut self, settings: &NrcSettings) -> Result<()> {
            self.device.wait_idle();
            self.buffers.clear();
            self.settings = None;
            let mut raw = [RawBuffer::default(); NrcBuffer::ALL.len()];
            let library = self.library.lock();
            // SAFETY: the device is idle, so nothing uses the buffers NRC frees; `raw` holds one
            // entry per buffer.
            let code = unsafe {
                (library.configure)(
                    library.context,
                    settings,
                    raw.as_mut_ptr(),
                    raw.len() as u32,
                )
            };
            status("forge_nrc_configure", code)?;
            // The shaders read each buffer from its address: one without (or not at its buffer's
            // start) would be read wrong.
            if let Some((i, _)) = raw
                .iter()
                .enumerate()
                .find(|(_, b)| b.size > 0 && (b.address == 0 || b.offset != 0))
            {
                return Err(GpuError::Nrc(format!(
                    "NRC's {:?} buffer has no address of its own",
                    NrcBuffer::ALL[i]
                )));
            }
            self.buffers = raw
                .iter()
                .map(|b| {
                    (b.buffer != 0 && b.size > 0).then(|| {
                        GraphBuffer::new(Buffer::foreign(
                            &self.device,
                            vk::Buffer::from_raw(b.buffer),
                            b.size,
                            b.address,
                            MemoryCategory::Work,
                        ))
                    })
                })
                .collect();
            self.settings = Some(*settings);
            let bytes: u64 = raw.iter().map(|b| b.size).sum();
            tracing::info!(
                frame = ?settings.frame,
                training = ?settings.training,
                mib = bytes / (1 << 20),
                "NRC configured"
            );
            Ok(())
        }

        pub(in crate::nrc) fn settings(&self) -> Option<&NrcSettings> {
            self.settings.as_ref()
        }

        pub(in crate::nrc) fn buffer(&self, which: NrcBuffer) -> Option<&GraphBuffer> {
            self.buffers.get(which as usize).and_then(Option::as_ref)
        }

        pub(in crate::nrc) fn begin_frame(
            &self,
            commands: &Commands<'_>,
            frame: &NrcFrameSettings,
        ) -> Result<[u8; CONSTANTS_BYTES]> {
            if self.settings.is_none() {
                return Err(GpuError::Nrc("begin_frame before configure".into()));
            }
            let library = self.library.lock();
            let mut constants = [0u8; CONSTANTS_BYTES];
            // SAFETY: the command buffer is recording inside a graph pass; `constants` holds
            // `NrcConstants` (its size checked at load).
            let code = unsafe {
                (library.begin_frame)(
                    library.context,
                    commands.raw(),
                    frame,
                    constants.as_mut_ptr().cast(),
                )
            };
            status("forge_nrc_begin_frame", code)?;
            self.open.store(true, Ordering::Relaxed);
            Ok(constants)
        }

        pub(in crate::nrc) fn query_and_train(&self, commands: &Commands<'_>) -> Result<()> {
            let library = self.library.lock();
            // SAFETY: the command buffer is recording inside a graph pass that declared NRC's
            // buffers.
            let code = unsafe { (library.query_and_train)(library.context, commands.raw()) };
            status("forge_nrc_query_and_train", code)
        }

        pub(in crate::nrc) fn end_frame(&self) -> Result<()> {
            if !self.open.swap(false, Ordering::Relaxed) {
                return Ok(());
            }
            let library = self.library.lock();
            let _queues = self.device.hold_queues();
            // SAFETY: the frame's command buffer was submitted to the graphics queue, which the
            // held lock keeps to this thread.
            let code =
                unsafe { (library.end_frame)(library.context, self.device.graphics_queue()) };
            status("forge_nrc_end_frame", code)
        }
    }

    impl Drop for Nrc {
        fn drop(&mut self) {
            // NRC frees its buffers with its context: nothing may use them.
            self.device.wait_idle();
            self.buffers.clear();
        }
    }
}

#[cfg(not(all(feature = "nrc", windows)))]
mod imp {
    use std::path::Path;
    use std::sync::Arc;

    use ash::vk;

    use super::{CONSTANTS_BYTES, NrcBuffer, NrcFrameSettings, NrcSettings};
    use crate::Device;
    use crate::commands::Commands;
    use crate::error::{GpuError, Result};
    use crate::graph::GraphBuffer;

    /// Never constructed without the `nrc` feature on Windows.
    pub(in crate::nrc) struct Nrc(std::convert::Infallible);

    impl Nrc {
        pub(in crate::nrc) fn load(_: &Arc<Device>, _: &Path) -> Result<Self> {
            Err(GpuError::Nrc(
                "built without the `nrc` feature (Windows only)".into(),
            ))
        }

        pub(in crate::nrc) fn training_size(&self, _: vk::Extent2D, _: u32) -> vk::Extent2D {
            match self.0 {}
        }

        pub(in crate::nrc) fn configure(&mut self, _: &NrcSettings) -> Result<()> {
            match self.0 {}
        }

        pub(in crate::nrc) fn settings(&self) -> Option<&NrcSettings> {
            match self.0 {}
        }

        pub(in crate::nrc) fn buffer(&self, _: NrcBuffer) -> Option<&GraphBuffer> {
            match self.0 {}
        }

        pub(in crate::nrc) fn begin_frame(
            &self,
            _: &Commands<'_>,
            _: &NrcFrameSettings,
        ) -> Result<[u8; CONSTANTS_BYTES]> {
            match self.0 {}
        }

        pub(in crate::nrc) fn query_and_train(&self, _: &Commands<'_>) -> Result<()> {
            match self.0 {}
        }

        pub(in crate::nrc) fn end_frame(&self) -> Result<()> {
            match self.0 {}
        }
    }
}

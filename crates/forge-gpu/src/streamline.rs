//! NVIDIA Streamline, for DLSS Super Resolution (issue #8), lifted from the `world` project.
//! Built with the `dlss` feature on Windows; the SDK itself stays outside the repository
//! (`streamline-sdk/bin/x64`, git-ignored). The safe face of it is [`crate::dlss`].
//!
//! Streamline sits between the application and the Vulkan loader: `sl.interposer.dll` is loaded in
//! place of `vulkan-1.dll`, and its `vkCreateInstance` and `vkCreateDevice` add what DLSS needs
//! while its swapchain functions keep the frame bookkeeping. The `sl*` functions are called through
//! the declarations below, which mirror the C++ headers of Streamline 2.14 (`sl_core_types.h`,
//! `sl_consts.h`, `sl_dlss.h`): every structure starts with a `BaseStructure` and its layout is
//! checked against the sizes MSVC gives the originals.

use std::ffi::{CStr, c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{GpuError, Result};

/// `sl::kSDKVersion` of Streamline 2.14.1.
const SDK_VERSION: u64 = (2 << 48) | (14 << 32) | (1 << 16) | 0xfedc;
pub(crate) const FEATURE_DLSS: u32 = 0;
/// Identifies the project to NGX in place of an application id NVIDIA would give.
const PROJECT_ID: &CStr = c"cbe477df-7d1e-4cce-9bfd-42c06a933d93";
const ENGINE_VERSION: &CStr = c"0.1";

pub(crate) const BUFFER_DEPTH: u32 = 0;
pub(crate) const BUFFER_MOTION_VECTORS: u32 = 1;
pub(crate) const BUFFER_SCALING_INPUT_COLOR: u32 = 3;
pub(crate) const BUFFER_SCALING_OUTPUT_COLOR: u32 = 4;
pub(crate) const BUFFER_EXPOSURE: u32 = 13;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StructType {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct BaseStructure {
    next: *mut BaseStructure,
    struct_type: StructType,
    struct_version: usize,
}

impl BaseStructure {
    const fn new(struct_type: StructType, struct_version: usize) -> Self {
        Self {
            next: std::ptr::null_mut(),
            struct_type,
            struct_version,
        }
    }
}

const PREFERENCES: StructType = StructType {
    data1: 0x1ca1_0965,
    data2: 0xbf8e,
    data3: 0x432b,
    data4: [0x8d, 0xa1, 0x67, 0x16, 0xd8, 0x79, 0xfb, 0x14],
};
const ADAPTER_INFO: StructType = StructType {
    data1: 0x0677_315f,
    data2: 0xa746,
    data3: 0x4492,
    data4: [0x9f, 0x42, 0xcb, 0x61, 0x42, 0xc9, 0xc3, 0xd4],
};
const VIEWPORT_HANDLE: StructType = StructType {
    data1: 0x171b_6435,
    data2: 0x9b3c,
    data3: 0x4fc8,
    data4: [0x99, 0x94, 0xfb, 0xe5, 0x25, 0x69, 0xaa, 0xa4],
};
const RESOURCE: StructType = StructType {
    data1: 0x3a9d_70cf,
    data2: 0x2418,
    data3: 0x4b72,
    data4: [0x83, 0x91, 0x13, 0xf8, 0x72, 0x1c, 0x72, 0x61],
};
const RESOURCE_TAG: StructType = StructType {
    data1: 0x4c6a_5aad,
    data2: 0xb445,
    data3: 0x496c,
    data4: [0x87, 0xff, 0x1a, 0xf3, 0x84, 0x5b, 0xe6, 0x53],
};
const CONSTANTS: StructType = StructType {
    data1: 0xdcd3_5ad7,
    data2: 0x4e4a,
    data3: 0x4bad,
    data4: [0xa9, 0x0c, 0xe0, 0xc4, 0x9e, 0xb2, 0x3a, 0xfe],
};
const DLSS_OPTIONS: StructType = StructType {
    data1: 0x6ac8_26e4,
    data2: 0x4c61,
    data3: 0x4101,
    data4: [0xa9, 0x2d, 0x63, 0x8d, 0x42, 0x10, 0x57, 0xb8],
};
const DLSS_OPTIMAL_SETTINGS: StructType = StructType {
    data1: 0xef1d_0957,
    data2: 0xfd58,
    data3: 0x4df7,
    data4: [0xb5, 0x04, 0x8b, 0x69, 0xd8, 0xaa, 0x6b, 0x76],
};

#[repr(C)]
struct Preferences {
    base: BaseStructure,
    show_console: bool,
    log_level: u32,
    paths_to_plugins: *const *const u16,
    num_paths_to_plugins: u32,
    path_to_logs_and_data: *const u16,
    allocate_callback: *const c_void,
    release_callback: *const c_void,
    log_message_callback: Option<unsafe extern "C" fn(u32, *const c_char)>,
    flags: u64,
    features_to_load: *const u32,
    num_features_to_load: u32,
    application_id: u32,
    engine: u32,
    engine_version: *const c_char,
    project_id: *const c_char,
    render_api: u32,
}

#[repr(C)]
struct AdapterInfo {
    base: BaseStructure,
    device_luid: *mut u8,
    device_luid_size_in_bytes: u32,
    vk_physical_device: *mut c_void,
}

/// `sl::ViewportHandle`: the one viewport this renderer upscales.
#[repr(C)]
pub(crate) struct ViewportHandle {
    base: BaseStructure,
    value: u32,
}

impl ViewportHandle {
    pub(crate) const fn new(value: u32) -> Self {
        Self {
            base: BaseStructure::new(VIEWPORT_HANDLE, 1),
            value,
        }
    }

    pub(crate) fn as_base(&self) -> *const BaseStructure {
        std::ptr::from_ref(self).cast()
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Extent {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
}

/// `sl::Resource` for a Vulkan image.
#[repr(C)]
pub(crate) struct Resource {
    base: BaseStructure,
    kind: c_char,
    native: *mut c_void,
    memory: *mut c_void,
    view: *mut c_void,
    state: u32,
    width: u32,
    height: u32,
    native_format: u32,
    mip_levels: u32,
    array_layers: u32,
    gpu_virtual_address: u64,
    flags: u32,
    usage: u32,
    internal_flags: u16,
    reserved: u16,
}

/// What Streamline needs to know about a Vulkan image to use it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VulkanImage {
    pub image: u64,
    pub view: u64,
    pub memory: u64,
    pub layout: i32,
    pub format: i32,
    pub usage: u32,
    pub width: u32,
    pub height: u32,
}

impl Resource {
    pub(crate) fn image(image: VulkanImage) -> Self {
        Self {
            base: BaseStructure::new(RESOURCE, 1),
            kind: 0,
            native: image.image as *mut c_void,
            memory: image.memory as *mut c_void,
            view: image.view as *mut c_void,
            state: image.layout as u32,
            width: image.width,
            height: image.height,
            native_format: image.format as u32,
            mip_levels: 1,
            array_layers: 1,
            gpu_virtual_address: 0,
            flags: 0,
            usage: image.usage,
            internal_flags: 0,
            reserved: 0,
        }
    }
}

#[repr(C)]
pub(crate) struct ResourceTag {
    base: BaseStructure,
    resource: *mut Resource,
    buffer_type: u32,
    lifecycle: i32,
    extent: Extent,
}

impl ResourceTag {
    /// A tag for `resource`, which stays as it is until the frame is presented
    /// (`eValidUntilPresent`): Streamline uses it in place. `eOnlyValidNow` made it copy every
    /// input first (and ask for transfer usage the render graph had not declared).
    pub(crate) fn until_present(resource: &mut Resource, buffer_type: u32, extent: Extent) -> Self {
        Self {
            base: BaseStructure::new(RESOURCE_TAG, 1),
            resource,
            buffer_type,
            lifecycle: 1,
            extent,
        }
    }
}

/// `sl::float4x4`: rows of a matrix applied to row vectors, so a column-major matrix (`glam`)
/// goes in as its columns.
pub(crate) type Matrix = [f32; 16];

/// `sl::Boolean`.
pub(crate) const FALSE: c_char = 0;
pub(crate) const TRUE: c_char = 1;

#[repr(C)]
pub(crate) struct Constants {
    pub base: BaseStructure,
    pub camera_view_to_clip: Matrix,
    pub clip_to_camera_view: Matrix,
    pub clip_to_lens_clip: Matrix,
    pub clip_to_prev_clip: Matrix,
    pub prev_clip_to_clip: Matrix,
    pub jitter_offset: [f32; 2],
    pub mvec_scale: [f32; 2],
    pub camera_pinhole_offset: [f32; 2],
    pub camera_pos: [f32; 3],
    pub camera_up: [f32; 3],
    pub camera_right: [f32; 3],
    pub camera_fwd: [f32; 3],
    pub camera_near: f32,
    pub camera_far: f32,
    pub camera_fov: f32,
    pub camera_aspect_ratio: f32,
    pub motion_vectors_invalid_value: f32,
    pub depth_inverted: c_char,
    pub camera_motion_included: c_char,
    pub motion_vectors_3d: c_char,
    pub reset: c_char,
    pub orthographic_projection: c_char,
    pub motion_vectors_dilated: c_char,
    pub motion_vectors_jittered: c_char,
    pub min_relative_linear_depth_object_separation: f32,
}

impl Default for Constants {
    fn default() -> Self {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        Self {
            base: BaseStructure::new(CONSTANTS, 2),
            camera_view_to_clip: identity,
            clip_to_camera_view: identity,
            clip_to_lens_clip: identity,
            clip_to_prev_clip: identity,
            prev_clip_to_clip: identity,
            jitter_offset: [0.0; 2],
            mvec_scale: [1.0; 2],
            camera_pinhole_offset: [0.0; 2],
            camera_pos: [0.0; 3],
            camera_up: [0.0, 1.0, 0.0],
            camera_right: [1.0, 0.0, 0.0],
            camera_fwd: [0.0, 0.0, -1.0],
            camera_near: f32::MAX,
            camera_far: f32::MAX,
            camera_fov: f32::MAX,
            camera_aspect_ratio: f32::MAX,
            motion_vectors_invalid_value: f32::MAX,
            depth_inverted: 2,
            camera_motion_included: 2,
            motion_vectors_3d: 2,
            reset: 2,
            orthographic_projection: FALSE,
            motion_vectors_dilated: FALSE,
            motion_vectors_jittered: FALSE,
            min_relative_linear_depth_object_separation: 40.0,
        }
    }
}

#[repr(C)]
pub(crate) struct DlssOptions {
    base: BaseStructure,
    pub mode: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub sharpness: f32,
    pub pre_exposure: f32,
    pub exposure_scale: f32,
    pub color_buffers_hdr: c_char,
    pub indicator_invert_axis_x: c_char,
    pub indicator_invert_axis_y: c_char,
    pub dlaa_preset: u32,
    pub quality_preset: u32,
    pub balanced_preset: u32,
    pub performance_preset: u32,
    pub ultra_performance_preset: u32,
    pub ultra_quality_preset: u32,
    pub use_auto_exposure: c_char,
    pub alpha_upscaling_enabled: c_char,
}

impl DlssOptions {
    /// Options for `mode` (an `sl::DLSSMode` value) at an output of `output` pixels, the input
    /// colour pre-exposed by `pre_exposure`.
    pub(crate) fn new(mode: u32, output: [u32; 2], pre_exposure: f32) -> Self {
        Self {
            base: BaseStructure::new(DLSS_OPTIONS, 3),
            mode,
            output_width: output[0],
            output_height: output[1],
            sharpness: 0.0,
            pre_exposure,
            exposure_scale: 1.0,
            color_buffers_hdr: TRUE,
            indicator_invert_axis_x: FALSE,
            indicator_invert_axis_y: FALSE,
            dlaa_preset: 0,
            quality_preset: 0,
            balanced_preset: 0,
            performance_preset: 0,
            ultra_performance_preset: 0,
            ultra_quality_preset: 0,
            // The exposure is given as a 1 × 1 texture (the scene is pre-exposed: 1).
            use_auto_exposure: FALSE,
            alpha_upscaling_enabled: FALSE,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub(crate) struct DlssOptimalSettings {
    base: BaseStructure,
    pub optimal_render_width: u32,
    pub optimal_render_height: u32,
    pub optimal_sharpness: f32,
    pub render_width_min: u32,
    pub render_height_min: u32,
    pub render_width_max: u32,
    pub render_height_max: u32,
}

impl Default for DlssOptimalSettings {
    fn default() -> Self {
        Self {
            base: BaseStructure::new(DLSS_OPTIMAL_SETTINGS, 1),
            optimal_render_width: 0,
            optimal_render_height: 0,
            optimal_sharpness: 0.0,
            render_width_min: 0,
            render_height_min: 0,
            render_width_max: 0,
            render_height_max: 0,
        }
    }
}

// Sizes MSVC gives the C++ structures (x64): a mismatch would corrupt every call.
const _: () = {
    assert!(size_of::<BaseStructure>() == 32);
    assert!(size_of::<Preferences>() == 144);
    assert!(size_of::<AdapterInfo>() == 56);
    assert!(size_of::<ViewportHandle>() == 40);
    assert!(size_of::<Resource>() == 112);
    assert!(size_of::<ResourceTag>() == 64);
    assert!(size_of::<Constants>() == 456);
    assert!(size_of::<DlssOptions>() == 88);
    assert!(size_of::<DlssOptimalSettings>() == 64);
    // Offsets where padding decides, worked out from the headers.
    assert!(std::mem::offset_of!(Preferences, log_level) == 36);
    assert!(std::mem::offset_of!(Preferences, flags) == 88);
    assert!(std::mem::offset_of!(Preferences, render_api) == 136);
    assert!(std::mem::offset_of!(Resource, native) == 40);
    assert!(std::mem::offset_of!(Resource, state) == 64);
    assert!(std::mem::offset_of!(Resource, gpu_virtual_address) == 88);
    assert!(std::mem::offset_of!(Resource, internal_flags) == 104);
    assert!(std::mem::offset_of!(ResourceTag, extent) == 48);
    assert!(std::mem::offset_of!(Constants, jitter_offset) == 352);
    assert!(std::mem::offset_of!(Constants, camera_near) == 424);
    assert!(std::mem::offset_of!(Constants, depth_inverted) == 444);
    assert!(std::mem::offset_of!(Constants, min_relative_linear_depth_object_separation) == 452);
    assert!(std::mem::offset_of!(DlssOptions, color_buffers_hdr) == 56);
    assert!(std::mem::offset_of!(DlssOptions, dlaa_preset) == 60);
    assert!(std::mem::offset_of!(DlssOptions, use_auto_exposure) == 84);
};

type SlResult = i32;
type Init = unsafe extern "C" fn(*const Preferences, u64) -> SlResult;
type Shutdown = unsafe extern "C" fn() -> SlResult;
type IsFeatureSupported = unsafe extern "C" fn(u32, *const AdapterInfo) -> SlResult;
type GetNewFrameToken = unsafe extern "C" fn(*mut *mut c_void, *const u32) -> SlResult;
type SetConstants =
    unsafe extern "C" fn(*const Constants, *const c_void, *const ViewportHandle) -> SlResult;
type SetTagForFrame = unsafe extern "C" fn(
    *const c_void,
    *const ViewportHandle,
    *const ResourceTag,
    u32,
    *mut c_void,
) -> SlResult;
type EvaluateFeature = unsafe extern "C" fn(
    u32,
    *const c_void,
    *const *const BaseStructure,
    u32,
    *mut c_void,
) -> SlResult;
type GetFeatureFunction = unsafe extern "C" fn(u32, *const c_char, *mut *mut c_void) -> SlResult;
type DlssGetOptimalSettings =
    unsafe extern "C" fn(*const DlssOptions, *mut DlssOptimalSettings) -> SlResult;
type DlssSetOptions = unsafe extern "C" fn(*const ViewportHandle, *const DlssOptions) -> SlResult;

const RESULT_NAMES: [&str; 40] = [
    "eOk",
    "eErrorIO",
    "eErrorDriverOutOfDate",
    "eErrorOSOutOfDate",
    "eErrorOSDisabledHWS",
    "eErrorDeviceNotCreated",
    "eErrorNoSupportedAdapterFound",
    "eErrorAdapterNotSupported",
    "eErrorNoPlugins",
    "eErrorVulkanAPI",
    "eErrorDXGIAPI",
    "eErrorD3DAPI",
    "eErrorNRDAPI",
    "eErrorNVAPI",
    "eErrorReflexAPI",
    "eErrorNGXFailed",
    "eErrorJSONParsing",
    "eErrorMissingProxy",
    "eErrorMissingResourceState",
    "eErrorInvalidIntegration",
    "eErrorMissingInputParameter",
    "eErrorNotInitialized",
    "eErrorComputeFailed",
    "eErrorInitNotCalled",
    "eErrorExceptionHandler",
    "eErrorInvalidParameter",
    "eErrorMissingConstants",
    "eErrorDuplicatedConstants",
    "eErrorMissingOrInvalidAPI",
    "eErrorCommonConstantsMissing",
    "eErrorUnsupportedInterface",
    "eErrorFeatureMissing",
    "eErrorFeatureNotSupported",
    "eErrorFeatureMissingHooks",
    "eErrorFeatureFailedToLoad",
    "eErrorFeatureWrongPriority",
    "eErrorFeatureMissingDependency",
    "eErrorFeatureManagerInvalidState",
    "eErrorInvalidState",
    "eWarnOutOfVRAM",
];

fn check(call: &'static str, result: SlResult) -> Result<()> {
    if result == 0 {
        return Ok(());
    }
    let name = usize::try_from(result)
        .ok()
        .and_then(|index| RESULT_NAMES.get(index))
        .copied()
        .unwrap_or("unknown");
    Err(GpuError::Streamline(format!("{call}: {name} ({result})")))
}

unsafe extern "C" fn log_message(kind: u32, message: *const c_char) {
    if message.is_null() {
        return;
    }
    // SAFETY: Streamline passes a nul-terminated string valid for the call.
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    let text = text.trim_end();
    match kind {
        2 => tracing::error!(target: "streamline", "{text}"),
        1 => tracing::warn!(target: "streamline", "{text}"),
        _ => tracing::debug!(target: "streamline", "{text}"),
    }
}

/// The loaded Streamline library, initialized for DLSS on Vulkan.
pub(crate) struct Streamline {
    interposer: PathBuf,
    shutdown: Shutdown,
    is_feature_supported: IsFeatureSupported,
    get_new_frame_token: GetNewFrameToken,
    set_constants: SetConstants,
    set_tag_for_frame: SetTagForFrame,
    evaluate_feature: EvaluateFeature,
    get_feature_function: GetFeatureFunction,
    shut_down: AtomicBool,
    // Last: the functions above point into it.
    _library: libloading::Library,
}

// SAFETY: Streamline's functions may be called from any thread; the ones this renderer uses are
// called from the thread that records frames.
unsafe impl Send for Streamline {}
// SAFETY: as above; nothing in the struct is mutated without the atomic.
unsafe impl Sync for Streamline {}

impl Streamline {
    /// Loads `sl.interposer.dll` from `bin` (the SDK's `bin/x64`), where its plugins and DLSS
    /// live too, and initializes it for DLSS on Vulkan. Call before creating the Vulkan instance.
    pub(crate) fn load(bin: &Path) -> Result<Self> {
        let interposer = bin.join("sl.interposer.dll");
        // SAFETY: loading the NVIDIA-signed interposer runs its initialization code, trusted like
        // the Vulkan loader it stands in for.
        let library = unsafe { libloading::Library::new(&interposer) }.map_err(|error| {
            GpuError::Streamline(format!("cannot load {}: {error}", interposer.display()))
        })?;
        macro_rules! function {
            ($name:literal, $kind:ty) => {{
                // SAFETY: the symbol is one of Streamline's exported C functions, declared with
                // the signature of `sl_core_api.h`.
                let symbol = unsafe { library.get::<$kind>($name) }.map_err(|error| {
                    GpuError::Streamline(format!("{}: {error}", String::from_utf8_lossy($name)))
                })?;
                *symbol
            }};
        }
        let init: Init = function!(b"slInit\0", Init);
        let streamline = Self {
            interposer,
            shutdown: function!(b"slShutdown\0", Shutdown),
            is_feature_supported: function!(b"slIsFeatureSupported\0", IsFeatureSupported),
            get_new_frame_token: function!(b"slGetNewFrameToken\0", GetNewFrameToken),
            set_constants: function!(b"slSetConstants\0", SetConstants),
            set_tag_for_frame: function!(b"slSetTagForFrame\0", SetTagForFrame),
            evaluate_feature: function!(b"slEvaluateFeature\0", EvaluateFeature),
            get_feature_function: function!(b"slGetFeatureFunction\0", GetFeatureFunction),
            shut_down: AtomicBool::new(false),
            _library: library,
        };

        let plugins: Vec<u16> = bin.as_os_str().encode_wide().chain([0]).collect();
        let paths = [plugins.as_ptr()];
        let features = [FEATURE_DLSS];
        let preferences = Preferences {
            base: BaseStructure::new(PREFERENCES, 1),
            show_console: false,
            log_level: 1,
            paths_to_plugins: paths.as_ptr(),
            num_paths_to_plugins: 1,
            path_to_logs_and_data: std::ptr::null(),
            allocate_callback: std::ptr::null(),
            release_callback: std::ptr::null(),
            log_message_callback: Some(log_message),
            // No state tracking of the command buffers (the render graph binds what it needs
            // after every pass), tags given per frame, and no plugins downloaded over the air.
            flags: (1 << 0) | (1 << 7),
            features_to_load: features.as_ptr(),
            num_features_to_load: 1,
            application_id: 0,
            engine: 0,
            engine_version: ENGINE_VERSION.as_ptr(),
            project_id: PROJECT_ID.as_ptr(),
            render_api: 2,
        };
        // SAFETY: the preferences and everything they point to outlive the call, which copies them.
        check("slInit", unsafe { init(&preferences, SDK_VERSION) })?;
        tracing::info!(interposer = %streamline.interposer.display(), "Streamline initialized");
        Ok(streamline)
    }

    /// The library to load the Vulkan API from, in place of `vulkan-1.dll`.
    pub(crate) fn interposer(&self) -> &Path {
        &self.interposer
    }

    /// Whether DLSS runs on `physical_device` (a `VkPhysicalDevice` handle).
    pub(crate) fn dlss_supported(&self, physical_device: u64) -> Result<()> {
        let adapter = AdapterInfo {
            base: BaseStructure::new(ADAPTER_INFO, 1),
            device_luid: std::ptr::null_mut(),
            device_luid_size_in_bytes: 0,
            vk_physical_device: physical_device as *mut c_void,
        };
        // SAFETY: the adapter info outlives the call.
        check("slIsFeatureSupported", unsafe {
            (self.is_feature_supported)(FEATURE_DLSS, &adapter)
        })
    }

    /// A token for frame `index`, shared by every call about that frame.
    pub(crate) fn frame_token(&self, index: u32) -> Result<*mut c_void> {
        let mut token = std::ptr::null_mut();
        // SAFETY: both pointers are valid for the call; Streamline owns the token it returns.
        check("slGetNewFrameToken", unsafe {
            (self.get_new_frame_token)(&mut token, &index)
        })?;
        Ok(token)
    }

    pub(crate) fn set_constants(
        &self,
        constants: &Constants,
        token: *mut c_void,
        viewport: &ViewportHandle,
    ) -> Result<()> {
        // SAFETY: the token came from `frame_token`; the structures outlive the call.
        check("slSetConstants", unsafe {
            (self.set_constants)(constants, token, viewport)
        })
    }

    /// Tags resources for the frame of `token`, in the command buffer `commands` being recorded.
    ///
    /// # Safety
    /// The resources must be alive and in the layouts their tags give, and `commands` recording.
    pub(crate) unsafe fn set_tags(
        &self,
        token: *mut c_void,
        viewport: &ViewportHandle,
        tags: &[ResourceTag],
        commands: u64,
    ) -> Result<()> {
        // SAFETY: guaranteed by the caller; the tags outlive the call.
        check("slSetTagForFrame", unsafe {
            (self.set_tag_for_frame)(
                token,
                viewport,
                tags.as_ptr(),
                tags.len() as u32,
                commands as *mut c_void,
            )
        })
    }

    /// Records DLSS for the frame of `token` into `commands`.
    ///
    /// # Safety
    /// As for [`set_tags`](Self::set_tags), with the tags of this frame set.
    pub(crate) unsafe fn evaluate_dlss(
        &self,
        token: *mut c_void,
        viewport: &ViewportHandle,
        commands: u64,
    ) -> Result<()> {
        let inputs = [viewport.as_base()];
        // SAFETY: guaranteed by the caller; the inputs outlive the call.
        check("slEvaluateFeature", unsafe {
            (self.evaluate_feature)(
                FEATURE_DLSS,
                token,
                inputs.as_ptr(),
                1,
                commands as *mut c_void,
            )
        })
    }

    fn dlss_function(&self, name: &CStr) -> Result<*mut c_void> {
        let mut function = std::ptr::null_mut();
        // SAFETY: the name is nul-terminated and the out pointer valid.
        check("slGetFeatureFunction", unsafe {
            (self.get_feature_function)(FEATURE_DLSS, name.as_ptr(), &mut function)
        })?;
        if function.is_null() {
            return Err(GpuError::Streamline(format!(
                "DLSS has no {}",
                name.to_string_lossy()
            )));
        }
        Ok(function)
    }

    pub(crate) fn dlss_optimal_settings(
        &self,
        options: &DlssOptions,
    ) -> Result<DlssOptimalSettings> {
        let function = self.dlss_function(c"slDLSSGetOptimalSettings")?;
        // SAFETY: DLSS exports this function with the signature of `sl_dlss.h`.
        let function: DlssGetOptimalSettings = unsafe { std::mem::transmute(function) };
        let mut settings = DlssOptimalSettings::default();
        // SAFETY: both structures outlive the call.
        check("slDLSSGetOptimalSettings", unsafe {
            function(options, &mut settings)
        })?;
        Ok(settings)
    }

    pub(crate) fn dlss_set_options(
        &self,
        viewport: &ViewportHandle,
        options: &DlssOptions,
    ) -> Result<()> {
        let function = self.dlss_function(c"slDLSSSetOptions")?;
        // SAFETY: DLSS exports this function with the signature of `sl_dlss.h`.
        let function: DlssSetOptions = unsafe { std::mem::transmute(function) };
        // SAFETY: both structures outlive the call.
        check("slDLSSSetOptions", unsafe { function(viewport, options) })
    }

    /// Shuts Streamline down; call before the Vulkan device is destroyed. Later calls do nothing.
    pub(crate) fn shut_down(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        // SAFETY: called once, after the last frame and before the device goes.
        if let Err(error) = check("slShutdown", unsafe { (self.shutdown)() }) {
            tracing::warn!("{error}");
        }
    }
}

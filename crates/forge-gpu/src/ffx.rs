//! AMD's FSR through its signed DLLs (the FFX API of the FSR SDK 2.4.0,
//! `docs/research/fsr4-vulkan.md`): FSR 4's ML upscaler on the AMD cards that run it, and the
//! DLLs' FSR 3.1 wherever else they load. The loader (`amd_fidelityfx_loader_vk.dll`) finds the
//! upscaler's DLL beside itself and records its passes into the frame's command buffer, as DLSS
//! does through Streamline; Forge's own FSR 3.1 (`forge-render`, D-070) stays the anti-aliasing
//! where the DLLs are not used.
//!
//! Always compiled, so renderers and demos need no feature gates: without the `ffx` feature (or
//! off Windows) [`Ffx::load`] reports the DLLs missing.

use std::path::Path;

use ash::vk;

use crate::Device;
use crate::commands::Commands;
use crate::error::Result;
use crate::graph::ResolvedImage;

/// The upscaler's DLL loader's name.
pub const FFX_LOADER: &str = "amd_fidelityfx_loader_vk.dll";

/// The camera and settings of one evaluation.
#[derive(Clone, Copy, Debug)]
pub struct FfxFrame {
    /// The jitter in render pixels, as Forge's FSR 3.1 takes it.
    pub jitter: [f32; 2],
    /// The size the frame was drawn at.
    pub render: vk::Extent2D,
    /// The output's size.
    pub output: vk::Extent2D,
    /// Nothing on screen matches the frames before (a cut, a new size).
    pub reset: bool,
    /// Near plane in metres (reversed-Z, infinite far plane).
    pub near: f32,
    /// Vertical field of view, radians.
    pub vertical_fov: f32,
    /// Milliseconds since the last frame.
    pub frame_ms: f32,
    /// What the colour was multiplied by before it was drawn (the exposure).
    pub pre_exposure: f32,
    /// RCAS after the upscale, 0 to 1, or none.
    pub sharpness: Option<f32>,
}

/// What the upscaler reads and writes in one evaluation: images in the layouts their pass
/// declared (sampled ones in `sampled_layout`, the output as storage in `GENERAL`).
#[derive(Clone, Copy, Debug)]
pub struct FfxImages<'a> {
    /// The jittered HDR colour at the render size, linear.
    pub color: &'a ResolvedImage,
    /// Its depth (reversed-Z).
    pub depth: &'a ResolvedImage,
    /// Motion vectors at the render size: the previous frame's UV minus this one's, unjittered.
    pub motion: &'a ResolvedImage,
    /// A 1 × 1 `R32_SFLOAT` exposure.
    pub exposure: &'a ResolvedImage,
    /// FSR 3.1's reactive mask where the frame has one (FSR 4 does without).
    pub reactive: Option<&'a ResolvedImage>,
    /// The upscaled colour at the output size.
    pub output: &'a ResolvedImage,
}

/// The version [`Ffx::load`] makes its context with. The DLLs' own choice is the newest version
/// offered, a driver's (`"FSR 4.1.3 *"`) only when strictly newer than theirs; an override pins a
/// version and opts the context out of the drivers' updates (AMD's `ffx-api.md`), so it is given
/// only when their choice is not of the major version asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FfxPick {
    /// The DLLs' own choice.
    Default,
    /// Their choice when it is of this major version (4: FSR 4), else the newest of it where
    /// offered, else their choice.
    Prefer(u32),
    /// Their choice when it is of this major version, else the newest of it, else no context.
    Only(u32),
}

/// A version name's number, the parts it lacks 0: `[3, 1, 6]` for `"3.1.6"` (the DLLs' names),
/// `[4, 1, 3]` for `"FSR 4.1.3 *"` (a driver's).
pub fn version_number(name: &str) -> Option<[u32; 3]> {
    let digits = name.trim_start_matches(|c: char| !c.is_ascii_digit());
    let number = digits
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .next()?;
    let mut parts = number.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next().flatten()?;
    let minor = parts.next().flatten().unwrap_or(0);
    let patch = parts.next().flatten().unwrap_or(0);
    Some([major, minor, patch])
}

/// Which of the versions `names` [`FfxPick`] takes: `Ok(None)` for the DLLs' own choice,
/// `Ok(Some(i))` to override with the `i`-th, `Err(())` when `Only`'s major version is not
/// offered.
#[cfg_attr(not(all(feature = "ffx", windows)), allow(dead_code))]
fn pick_version(names: &[String], pick: FfxPick) -> std::result::Result<Option<usize>, ()> {
    let numbers: Vec<Option<[u32; 3]>> = names.iter().map(|n| version_number(n)).collect();
    // The newest of those `keep` lets through; on a tie the shipped one (no " *").
    let newest = |keep: &dyn Fn([u32; 3]) -> bool| {
        (0..names.len())
            .filter_map(|i| numbers[i].filter(|&v| keep(v)).map(|v| (i, v)))
            .max_by_key(|&(i, v)| (v, !names[i].trim_end().ends_with('*')))
            .map(|(i, _)| i)
    };
    let own = newest(&|_| true);
    let (FfxPick::Prefer(major) | FfxPick::Only(major)) = pick else {
        return Ok(None);
    };
    if own.and_then(|i| numbers[i]).is_some_and(|v| v[0] == major) {
        return Ok(None);
    }
    match newest(&|v| v[0] == major) {
        Some(i) => Ok(Some(i)),
        None if matches!(pick, FfxPick::Only(_)) => Err(()),
        None => Ok(None),
    }
}

/// The FFX upscaler on this device, one context for its maximum sizes.
pub struct Ffx(imp::Ffx);

impl Ffx {
    /// Loads [`FFX_LOADER`] from `dir` and makes a context for frames drawn at up to
    /// `max_render` and output at up to `max_output`, with the version `pick` asks for.
    pub fn load(
        device: &Device,
        dir: &Path,
        max_render: vk::Extent2D,
        max_output: vk::Extent2D,
        pick: FfxPick,
    ) -> Result<Self> {
        imp::Ffx::load(device, dir, max_render, max_output, pick).map(Self)
    }

    /// The versions the DLLs offer on this device (`"3.1.6"` and `"2.3.4"` on NVIDIA), and the
    /// one this context runs.
    pub fn versions(&self) -> (&[String], &str) {
        self.0.versions()
    }

    /// Records the upscale into `commands`, from inside a graph pass that declared every image
    /// of `images`. The DLLs restore each image's layout.
    pub fn dispatch(
        &self,
        commands: &Commands<'_>,
        frame: &FfxFrame,
        images: &FfxImages<'_>,
    ) -> Result<()> {
        self.0.dispatch(commands, frame, images)
    }
}

#[cfg(all(feature = "ffx", windows))]
mod imp {
    use std::ffi::{CStr, c_char, c_void};
    use std::path::Path;

    use ash::vk;
    use ash::vk::Handle;

    use super::{FFX_LOADER, FfxFrame, FfxImages, FfxPick, pick_version};
    use crate::Device;
    use crate::commands::Commands;
    use crate::error::{GpuError, Result};
    use crate::graph::ResolvedImage;

    // The FFX API's types (`ffx_api.h`, `ffx_api_types.h`, `ffx_upscale.h`, `vk/ffx_api_vk.h`).

    #[repr(C)]
    struct Header {
        kind: u64,
        next: *mut Header,
    }

    impl Header {
        fn new(kind: u64) -> Self {
            Self {
                kind,
                next: std::ptr::null_mut(),
            }
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Dimensions {
        width: u32,
        height: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Coords {
        x: f32,
        y: f32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ResourceDescription {
        kind: u32,
        format: u32,
        width: u32,
        height: u32,
        depth: u32,
        mips: u32,
        flags: u32,
        usage: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Resource {
        resource: *mut c_void,
        description: ResourceDescription,
        state: u32,
    }

    #[repr(C)]
    struct VkDeviceContext {
        device: vk::Device,
        physical: vk::PhysicalDevice,
        get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
    }

    #[repr(C)]
    struct CreateBackendVk {
        header: Header,
        context: VkDeviceContext,
    }

    type Message = unsafe extern "C" fn(kind: u32, message: *const u16);

    #[repr(C)]
    struct CreateUpscale {
        header: Header,
        flags: u32,
        max_render: Dimensions,
        max_upscale: Dimensions,
        message: Option<Message>,
    }

    #[repr(C)]
    struct CreateUpscaleVersion {
        header: Header,
        version: u32,
    }

    #[repr(C)]
    struct OverrideVersion {
        header: Header,
        version: u64,
    }

    #[repr(C)]
    struct QueryVersions {
        header: Header,
        create_kind: u64,
        device: *mut c_void,
        count: *mut u64,
        ids: *mut u64,
        names: *mut *const c_char,
    }

    #[repr(C)]
    struct QueryProviderVersion {
        header: Header,
        version_id: u64,
        version_name: *const c_char,
    }

    #[repr(C)]
    struct DispatchUpscale {
        header: Header,
        command_list: *mut c_void,
        color: Resource,
        depth: Resource,
        motion: Resource,
        exposure: Resource,
        reactive: Resource,
        transparency: Resource,
        output: Resource,
        jitter: Coords,
        motion_scale: Coords,
        render_size: Dimensions,
        upscale_size: Dimensions,
        sharpen: bool,
        sharpness: f32,
        frame_ms: f32,
        pre_exposure: f32,
        reset: bool,
        near: f32,
        far: f32,
        vertical_fov: f32,
        metres_per_unit: f32,
        flags: u32,
    }

    const RETURN_OK: u32 = 0;
    const EFFECT_UPSCALE: u64 = 0x0001_0000;
    const CREATE_UPSCALE: u64 = EFFECT_UPSCALE;
    const DISPATCH_UPSCALE: u64 = EFFECT_UPSCALE | 0x01;
    const CREATE_UPSCALE_VERSION: u64 = EFFECT_UPSCALE | 0x0b;
    const CREATE_BACKEND_VK: u64 = 0x0100_0003;
    const QUERY_GET_VERSIONS: u64 = 4;
    const OVERRIDE_VERSION: u64 = 5;
    const QUERY_GET_PROVIDER_VERSION: u64 = 6;
    /// `FFX_UPSCALER_VERSION` of the SDK the types above come from: 4.1.2.
    const UPSCALER_VERSION: u32 = (4 << 22) | (1 << 12) | 2;
    /// `FfxApiCreateContextUpscaleFlags`: HDR colour, reversed and infinite depth.
    const ENABLE_HIGH_DYNAMIC_RANGE: u32 = 1 << 0;
    const ENABLE_DEPTH_INVERTED: u32 = 1 << 3;
    const ENABLE_DEPTH_INFINITE: u32 = 1 << 4;
    const RESOURCE_TEXTURE2D: u32 = 2;
    const USAGE_READ_ONLY: u32 = 0;
    const USAGE_UAV: u32 = 1 << 1;
    const USAGE_DEPTHTARGET: u32 = 1 << 2;
    const STATE_COMMON: u32 = 1 << 0;
    const STATE_UNORDERED_ACCESS: u32 = 1 << 1;
    const STATE_COMPUTE_READ: u32 = 1 << 2;

    type CreateContext = unsafe extern "C" fn(*mut *mut c_void, *mut Header, *const c_void) -> u32;
    type DestroyContext = unsafe extern "C" fn(*mut *mut c_void, *const c_void) -> u32;
    type Query = unsafe extern "C" fn(*mut *mut c_void, *mut Header) -> u32;
    type Dispatch = unsafe extern "C" fn(*mut *mut c_void, *const Header) -> u32;

    /// The FFX API's surface format for `format` (`ffxApiGetSurfaceFormatVK`), for the formats
    /// Forge hands it; 0 (unknown) else.
    fn surface_format(format: vk::Format) -> u32 {
        match format {
            vk::Format::R32G32B32A32_SFLOAT => 3,
            vk::Format::R16G16B16A16_SFLOAT => 4,
            vk::Format::R32G32_SFLOAT => 6,
            vk::Format::B10G11R11_UFLOAT_PACK32 => 16,
            vk::Format::R16G16_SFLOAT => 18,
            vk::Format::R16_SFLOAT => 21,
            vk::Format::R8_UNORM => 25,
            vk::Format::R32_SFLOAT | vk::Format::D32_SFLOAT => 28,
            _ => 0,
        }
    }

    /// `image` as the FFX API takes it, in the state its layout is (`getVKImageLayoutFromResourceState`
    /// in the SDK's Vulkan backend: compute read is `SHADER_READ_ONLY_OPTIMAL`, common `GENERAL`).
    fn resource(image: &ResolvedImage, written: bool) -> Resource {
        let depth = image.format == vk::Format::D32_SFLOAT;
        let state = if written {
            STATE_UNORDERED_ACCESS
        } else if image.sampled_layout == vk::ImageLayout::GENERAL {
            STATE_COMMON
        } else {
            STATE_COMPUTE_READ
        };
        Resource {
            resource: image.raw.as_raw() as *mut c_void,
            description: ResourceDescription {
                kind: RESOURCE_TEXTURE2D,
                format: surface_format(image.format),
                width: image.extent.width,
                height: image.extent.height,
                depth: 1,
                mips: 1,
                flags: 0,
                usage: if written {
                    USAGE_UAV
                } else if depth {
                    USAGE_DEPTHTARGET
                } else {
                    USAGE_READ_ONLY
                },
            },
            state,
        }
    }

    fn none() -> Resource {
        Resource {
            resource: std::ptr::null_mut(),
            description: ResourceDescription {
                kind: 0,
                format: 0,
                width: 0,
                height: 0,
                depth: 0,
                mips: 0,
                flags: 0,
                usage: 0,
            },
            state: 0,
        }
    }

    /// The DLLs' messages, to the log.
    unsafe extern "C" fn message(kind: u32, text: *const u16) {
        if text.is_null() {
            return;
        }
        // SAFETY: the runtime passes a NUL-terminated wide string alive for the call.
        let text = unsafe {
            let mut n = 0;
            while *text.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(text, n))
        };
        if kind == 0 {
            tracing::warn!(target: "ffx", "{text}");
        } else {
            tracing::info!(target: "ffx", "{text}");
        }
    }

    pub(super) struct Ffx {
        // Kept loaded while the context lives.
        _library: libloading::Library,
        destroy: DestroyContext,
        dispatch: Dispatch,
        // The context's handle, an address (it never changes once made).
        context: usize,
        versions: Vec<String>,
        running: String,
    }

    impl Ffx {
        pub(super) fn load(
            device: &Device,
            dir: &Path,
            max_render: vk::Extent2D,
            max_output: vk::Extent2D,
            pick: FfxPick,
        ) -> Result<Self> {
            let path = dir.join(FFX_LOADER);
            if !path.exists() {
                return Err(GpuError::Ffx(format!("{} not found", path.display())));
            }
            // SAFETY: AMD's signed loader; its initialisation only registers its providers.
            let library = unsafe { libloading::Library::new(&path) }
                .map_err(|e| GpuError::Ffx(format!("cannot load {}: {e}", path.display())))?;
            macro_rules! function {
                ($name:literal, $kind:ty) => {{
                    // SAFETY: one of the loader's exported C functions (`ffx_api.h`), declared
                    // with its signature.
                    let symbol = unsafe { library.get::<$kind>($name) }.map_err(|e| {
                        GpuError::Ffx(format!("{}: {e}", String::from_utf8_lossy($name)))
                    })?;
                    *symbol
                }};
            }
            let create: CreateContext = function!(b"ffxCreateContext", CreateContext);
            let destroy: DestroyContext = function!(b"ffxDestroyContext", DestroyContext);
            let query: Query = function!(b"ffxQuery", Query);
            let dispatch: Dispatch = function!(b"ffxDispatch", Dispatch);

            let get_device_proc_addr = device.instance().raw().fp_v1_0().get_device_proc_addr;
            let mut context = VkDeviceContext {
                device: device.raw().handle(),
                physical: device.physical(),
                get_device_proc_addr,
            };
            // The versions offered, ids and names, copied at once (the SDK warns that later
            // queries may overwrite the names).
            let mut count = 0_u64;
            let mut versions = QueryVersions {
                header: Header::new(QUERY_GET_VERSIONS),
                create_kind: CREATE_UPSCALE,
                device: (&raw mut context).cast(),
                count: &raw mut count,
                ids: std::ptr::null_mut(),
                names: std::ptr::null_mut(),
            };
            // SAFETY: a query without a context (null), its output pointers alive for the call.
            let code = unsafe { query(std::ptr::null_mut(), &raw mut versions.header) };
            if code != RETURN_OK {
                return Err(GpuError::Ffx(format!("the version query failed ({code})")));
            }
            let mut ids = vec![0_u64; count as usize];
            let mut names = vec![std::ptr::null::<c_char>(); count as usize];
            versions.ids = ids.as_mut_ptr();
            versions.names = names.as_mut_ptr();
            // SAFETY: as above, the arrays as long as the count says.
            let code = unsafe { query(std::ptr::null_mut(), &raw mut versions.header) };
            if code != RETURN_OK {
                return Err(GpuError::Ffx(format!("the version query failed ({code})")));
            }
            let names: Vec<String> = names
                .iter()
                .map(|&n| {
                    if n.is_null() {
                        String::from("?")
                    } else {
                        // SAFETY: a NUL-terminated name the query wrote, alive until the next
                        // query.
                        unsafe { CStr::from_ptr(n) }.to_string_lossy().into_owned()
                    }
                })
                .collect();
            let chosen = pick_version(&names, pick).map_err(|()| {
                GpuError::Ffx(format!(
                    "no version asked for ({pick:?}) on this device (offered: {names:?})"
                ))
            })?;

            // The chain, made from its end: the version override (when one was chosen), the
            // API's version, the Vulkan backend, the upscaler's description.
            let mut over = OverrideVersion {
                header: Header::new(OVERRIDE_VERSION),
                version: chosen.map_or(0, |i| ids[i]),
            };
            let mut api = CreateUpscaleVersion {
                header: Header {
                    kind: CREATE_UPSCALE_VERSION,
                    next: if chosen.is_some() {
                        &raw mut over.header
                    } else {
                        std::ptr::null_mut()
                    },
                },
                version: UPSCALER_VERSION,
            };
            let mut backend = CreateBackendVk {
                header: Header {
                    kind: CREATE_BACKEND_VK,
                    next: &raw mut api.header,
                },
                context: VkDeviceContext {
                    device: device.raw().handle(),
                    physical: device.physical(),
                    get_device_proc_addr,
                },
            };
            let mut desc = CreateUpscale {
                header: Header::new(CREATE_UPSCALE),
                flags: ENABLE_HIGH_DYNAMIC_RANGE | ENABLE_DEPTH_INVERTED | ENABLE_DEPTH_INFINITE,
                max_render: Dimensions {
                    width: max_render.width,
                    height: max_render.height,
                },
                max_upscale: Dimensions {
                    width: max_output.width,
                    height: max_output.height,
                },
                message: Some(message),
            };
            desc.header.next = &raw mut backend.header;
            let mut handle: *mut c_void = std::ptr::null_mut();
            // SAFETY: the chain of descriptions alive for the call; the device is live.
            let code = unsafe { create(&raw mut handle, &raw mut desc.header, std::ptr::null()) };
            if code != RETURN_OK || handle.is_null() {
                return Err(GpuError::Ffx(format!(
                    "the upscaler's context failed ({code})"
                )));
            }
            // The version the context runs, as the DLLs say.
            let mut provider = QueryProviderVersion {
                header: Header::new(QUERY_GET_PROVIDER_VERSION),
                version_id: 0,
                version_name: std::ptr::null(),
            };
            let mut context = handle;
            // SAFETY: a query on the live context, its output fields alive for the call.
            let code = unsafe { query(&raw mut context, &raw mut provider.header) };
            let running = if code == RETURN_OK && !provider.version_name.is_null() {
                // SAFETY: a NUL-terminated name the query wrote, copied at once.
                unsafe { CStr::from_ptr(provider.version_name) }
                    .to_string_lossy()
                    .into_owned()
            } else {
                chosen.map(|i| names[i].clone()).unwrap_or_default()
            };
            Ok(Self {
                _library: library,
                destroy,
                dispatch,
                context: handle as usize,
                versions: names,
                running,
            })
        }

        pub(super) fn versions(&self) -> (&[String], &str) {
            (&self.versions, &self.running)
        }

        pub(super) fn dispatch(
            &self,
            commands: &Commands<'_>,
            frame: &FfxFrame,
            images: &FfxImages<'_>,
        ) -> Result<()> {
            let mut desc = DispatchUpscale {
                header: Header::new(DISPATCH_UPSCALE),
                command_list: commands.raw().as_raw() as *mut c_void,
                color: resource(images.color, false),
                depth: resource(images.depth, false),
                motion: resource(images.motion, false),
                exposure: resource(images.exposure, false),
                reactive: images.reactive.map_or_else(none, |r| resource(r, false)),
                transparency: none(),
                output: resource(images.output, true),
                jitter: Coords {
                    x: frame.jitter[0],
                    y: frame.jitter[1],
                },
                // Forge's motion is in UV: the API wants its scale to pixels.
                motion_scale: Coords {
                    x: frame.render.width as f32,
                    y: frame.render.height as f32,
                },
                render_size: Dimensions {
                    width: frame.render.width,
                    height: frame.render.height,
                },
                upscale_size: Dimensions {
                    width: frame.output.width,
                    height: frame.output.height,
                },
                sharpen: frame.sharpness.is_some(),
                sharpness: frame.sharpness.unwrap_or(0.0),
                frame_ms: frame.frame_ms,
                pre_exposure: frame.pre_exposure.max(1e-6),
                reset: frame.reset,
                near: frame.near,
                far: f32::MAX,
                vertical_fov: frame.vertical_fov,
                metres_per_unit: 1.0,
                flags: 0,
            };
            let mut context = self.context as *mut c_void;
            // SAFETY: the context is live, the command buffer recording, every image declared
            // by the pass in the state given.
            let code = unsafe { (self.dispatch)(&raw mut context, &raw mut desc.header) };
            if code != RETURN_OK {
                return Err(GpuError::Ffx(format!("the upscale failed ({code})")));
            }
            Ok(())
        }
    }

    impl Drop for Ffx {
        fn drop(&mut self) {
            let mut context = self.context as *mut c_void;
            // SAFETY: the context made in `load`, no frame using it any more (the owner waits
            // for the device before dropping its upscaler).
            unsafe { (self.destroy)(&raw mut context, std::ptr::null()) };
        }
    }
}

#[cfg(not(all(feature = "ffx", windows)))]
mod imp {
    use std::path::Path;

    use ash::vk;

    use super::{FfxFrame, FfxImages, FfxPick};
    use crate::Device;
    use crate::commands::Commands;
    use crate::error::{GpuError, Result};

    /// Never constructed without the `ffx` feature on Windows.
    pub(super) struct Ffx(std::convert::Infallible);

    impl Ffx {
        pub(super) fn load(
            _: &Device,
            _: &Path,
            _: vk::Extent2D,
            _: vk::Extent2D,
            _: FfxPick,
        ) -> Result<Self> {
            Err(GpuError::Ffx(
                "built without the `ffx` feature (Windows only)".into(),
            ))
        }

        pub(super) fn versions(&self) -> (&[String], &str) {
            match self.0 {}
        }

        pub(super) fn dispatch(
            &self,
            _: &Commands<'_>,
            _: &FfxFrame,
            _: &FfxImages<'_>,
        ) -> Result<()> {
            match self.0 {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FfxPick, pick_version, version_number};

    /// The DLLs name their versions by number ("3.1.6", "2.3.4" on NVIDIA); a driver's name
    /// ("FSR 4.1.3 *") reads the same, and one without a number reads as none.
    #[test]
    fn a_versions_number_reads_from_its_name() {
        assert_eq!(version_number("3.1.6"), Some([3, 1, 6]));
        assert_eq!(version_number("2.3.4"), Some([2, 3, 4]));
        assert_eq!(version_number("FSR 4.1.3 *"), Some([4, 1, 3]));
        assert_eq!(version_number("10.1"), Some([10, 1, 0]));
        assert_eq!(version_number("?"), None);
    }

    /// An override only where the DLLs' own choice (the newest) is not of the major version
    /// asked for: FSR 4 left to them, and to the drivers' updates, wherever it is their choice.
    #[test]
    fn a_version_is_pinned_only_where_their_choice_is_not_the_one_asked_for() {
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let nvidia = names(&["3.1.6", "2.3.4"]);
        assert_eq!(pick_version(&nvidia, FfxPick::Default), Ok(None));
        assert_eq!(pick_version(&nvidia, FfxPick::Prefer(4)), Ok(None));
        assert_eq!(pick_version(&nvidia, FfxPick::Only(4)), Err(()));
        assert_eq!(pick_version(&nvidia, FfxPick::Only(2)), Ok(Some(1)));
        // Listed in any order, a driver's newer FSR 4 beside the shipped one: their choice.
        let amd = names(&["3.1.6", "4.1.2", "FSR 4.1.3 *", "2.3.4"]);
        assert_eq!(pick_version(&amd, FfxPick::Only(4)), Ok(None));
        assert_eq!(pick_version(&amd, FfxPick::Only(3)), Ok(Some(0)));
        // A major version offered twice: the newest of it.
        let two = names(&["5.0.0", "4.0.1", "4.1.2"]);
        assert_eq!(pick_version(&two, FfxPick::Prefer(4)), Ok(Some(2)));
    }
}

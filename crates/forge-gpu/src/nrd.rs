//! NVIDIA's NRD (Real-time Denoisers), its SIGMA shadow denoiser: the sun's ray-traced soft
//! shadows denoised by NVIDIA's own library (issue #172, D-049).
//!
//! NRD is never in the repository (the NVIDIA RTX SDKs License, as DLSS): `tools/fetch-nrd.sh`
//! builds the pinned release [`NRD_RELEASE`] into `nrd-sdk/bin`, git-ignored, and [`Sigma::load`]
//! loads that library at run time. Built without the `nrd` feature, or with no library there,
//! [`Sigma::load`] fails and the renderer keeps its own shadow ray, pixel for pixel as before.
//!
//! NRD makes no GPU calls: it hands back its compute shaders (SPIR-V) once, then every frame a
//! list of dispatches, each naming its pipeline, the images it reads and writes and its
//! constants. [`Sigma::denoise`] declares each dispatch as a render-graph pass (`shadow/…`), so
//! the graph derives the barriers and the profiler zones as for every other pass. The shaders
//! bind their own descriptor sets (NRD's register spaces), not the bindless set: set 0 holds the
//! dispatch's images, pushed with `VK_KHR_push_descriptor`; set 1 its two samplers and its
//! constants, at an offset of a ring of per-frame constants.
//!
//! NRD's kernels are written for D3D12, which drops a write outside an image and reads zero
//! there: several of them (its clears, SIGMA's history copy and tile smoothing) run their last
//! workgroup past the image's edge unchecked. In Vulkan that is undefined; on the RTX 5070 Ti the
//! clears' stray writes landed in the shadow ray's inputs, and the frame differed with
//! `FORGE_ASYNC=0`. So NRD's pipelines alone are created robust (`VK_EXT_pipeline_robustness`,
//! robust image access: D3D12's behaviour), Forge's own keep the device's default, and NRD's
//! clears become `vkCmdClearColorImage`, which clears the image exactly.
//!
//! The declarations below mirror NRD 4.18's C API (`NRD.h`, `NRDDescs.h`, `NRDSettings.h`):
//! every structure's size and the offsets where padding decides were checked against MSVC's
//! layout of the originals, and the library's version is checked when it loads.

use std::sync::Arc;

use ash::vk;

use crate::device::Device;
use crate::error::{GpuError, Result};
use crate::frame::FrameSlot;
use crate::graph::{FrameGraph, ImageHandle};

/// The NRD release `tools/fetch-nrd.sh` builds and [`Sigma::load`] accepts.
pub const NRD_RELEASE: &str = "v4.18.0";

/// The format of [`SigmaImages::penumbra`]: the penumbra's radius in metres where the shadow
/// ray met an occluder (hit distance × tan of the sun's angular radius, halved), 65 504 where
/// it reached the sun, 0 on a surface facing away from it.
pub const PENUMBRA_FORMAT: vk::Format = vk::Format::R32_SFLOAT;
/// The format of [`SigmaImages::normal_roughness`]: the world-space normal (xyz, any length)
/// and the linear roughness (w), as NRD is built to read them (`tools/fetch-nrd.sh`).
pub const NORMAL_ROUGHNESS_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
/// The format of [`SigmaImages::view_z`]: the view-space depth in metres (its sign does not
/// matter), beyond [`SigmaFrame::denoising_range`] where nothing was drawn.
pub const VIEW_Z_FORMAT: vk::Format = vk::Format::R32_SFLOAT;
/// The format of the denoised shadow: its square root (square it to get the visibility).
pub const SHADOW_FORMAT: vk::Format = vk::Format::R16_SFLOAT;

/// The images SIGMA reads, at the size it was loaded for. Every one needs `SAMPLED` usage.
#[derive(Clone, Copy, Debug)]
pub struct SigmaImages {
    /// [`PENUMBRA_FORMAT`].
    pub penumbra: ImageHandle,
    /// [`NORMAL_ROUGHNESS_FORMAT`].
    pub normal_roughness: ImageHandle,
    /// [`VIEW_Z_FORMAT`].
    pub view_z: ImageHandle,
    /// The motion vectors: the offset in UV to where each pixel was in the previous frame,
    /// without jitter (TAA's).
    pub motion: ImageHandle,
}

/// The camera and the sun of one frame. Matrices are column-major (`glam`'s `to_cols_array`)
/// and carry no jitter; "world" may be any frame that moves with the camera, as long as the
/// previous view includes the camera's step.
#[derive(Clone, Copy, Debug)]
pub struct SigmaFrame {
    /// View coordinates from world coordinates.
    pub world_to_view: [f32; 16],
    /// The previous frame's view from this frame's world coordinates.
    pub world_to_view_prev: [f32; 16],
    /// Clip coordinates from view coordinates (reversed-Z, infinite far plane).
    pub view_to_clip: [f32; 16],
    /// The previous frame's.
    pub view_to_clip_prev: [f32; 16],
    /// This frame's jitter in pixels, as NRD wants it: where the pixel's sample lies from its
    /// centre (x right, y down).
    pub jitter: [f32; 2],
    /// The previous frame's.
    pub jitter_prev: [f32; 2],
    /// The direction to the sun, in world coordinates.
    pub light_direction: [f32; 3],
    /// NRD's frame index: it turns the blur's taps. Forge passes the jitter's phase, so the
    /// taps repeat with TAA's cycle.
    pub frame_index: u32,
    /// The time from the previous frame, milliseconds (a fixed step in captures).
    pub time_delta_ms: f32,
    /// Nothing on screen matches the frames before (a cut, a new size).
    pub reset: bool,
    /// Pixels whose view depth is beyond this, in metres, are not denoised (the sky).
    pub denoising_range: f32,
    /// Frames the temporal stabilisation accumulates, at most 7 (0 turns it off).
    pub stabilized_frames: u32,
}

/// SIGMA, loaded for one image size (see the module's documentation).
pub struct Sigma(imp::Sigma);

impl Sigma {
    /// Loads NRD from `dir` (the folder holding `NRD.dll`, or `libNRD.so`), checks it is
    /// [`NRD_RELEASE`] built as Forge needs, and creates SIGMA's pipelines and images for
    /// `extent`. Fails without the `nrd` feature, without the library, or on a device without
    /// push descriptors and unformatted storage writes.
    pub fn load(device: &Arc<Device>, dir: &std::path::Path, extent: vk::Extent2D) -> Result<Self> {
        imp::Sigma::load(device, dir, extent).map(Self)
    }

    /// The image size it was loaded for.
    pub fn extent(&self) -> vk::Extent2D {
        self.0.extent()
    }

    /// The library's version, as it reports it.
    pub fn version(&self) -> String {
        self.0.version()
    }

    /// Declares SIGMA's dispatches for this frame as graph passes (`shadow/…`) and returns the
    /// denoised shadow ([`SHADOW_FORMAT`], sampled; it also serves as next frame's history).
    pub fn denoise<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        images: SigmaImages,
        frame: &SigmaFrame,
    ) -> Result<ImageHandle> {
        self.0.denoise(graph, slot, images, frame)
    }
}

#[cfg(not(feature = "nrd"))]
mod imp {
    use super::*;

    pub(super) enum Sigma {}

    impl Sigma {
        pub(super) fn load(
            _device: &Arc<Device>,
            _dir: &std::path::Path,
            _extent: vk::Extent2D,
        ) -> Result<Self> {
            Err(GpuError::Nrd("built without the `nrd` feature".into()))
        }

        pub(super) fn extent(&self) -> vk::Extent2D {
            match *self {}
        }

        pub(super) fn version(&self) -> String {
            match *self {}
        }

        pub(super) fn denoise<'f>(
            &'f self,
            _graph: &mut FrameGraph<'f>,
            _slot: FrameSlot,
            _images: SigmaImages,
            _frame: &SigmaFrame,
        ) -> Result<ImageHandle> {
            match *self {}
        }
    }
}

#[cfg(feature = "nrd")]
mod imp {
    use std::ffi::{CStr, c_char, c_void};

    use parking_lot::Mutex;

    use super::*;
    use crate::frame::FRAMES_IN_FLIGHT;
    use crate::graph::{GraphImage, ImageAccess};
    use crate::memory::{Buffer, BufferDesc, ImageDesc};
    use crate::memory_report::MemoryCategory;

    // ------------------------------------------------------------------ the C API ---

    #[repr(C)]
    struct AllocationCallbacks {
        allocate: *const c_void,
        reallocate: *const c_void,
        free: *const c_void,
        user_arg: *mut c_void,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SpirvBindingOffsets {
        sampler: u32,
        texture: u32,
        constant_buffer: u32,
        storage_texture_and_buffer: u32,
    }

    #[repr(C)]
    struct LibraryDesc {
        spirv_binding_offsets: SpirvBindingOffsets,
        supported_denoisers: *const u32,
        supported_denoisers_num: u32,
        version_major: u8,
        version_minor: u8,
        version_build: u8,
        normal_encoding: u8,
        roughness_encoding: u8,
    }

    #[repr(C)]
    struct DenoiserDesc {
        identifier: u32,
        denoiser: u32,
    }

    #[repr(C)]
    struct InstanceCreationDesc {
        allocation_callbacks: AllocationCallbacks,
        denoisers: *const DenoiserDesc,
        denoisers_num: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct TextureDesc {
        format: u32,
        downsample_factor: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ResourceDesc {
        descriptor_type: u32,
        resource_type: u32,
        index_in_pool: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ResourceRangeDesc {
        descriptor_type: u32,
        descriptors_num: u32,
    }

    #[repr(C)]
    struct ComputeShaderDesc {
        bytecode: *const c_void,
        size: u64,
    }

    #[repr(C)]
    struct PipelineDesc {
        compute_shader_dxbc: ComputeShaderDesc,
        compute_shader_dxil: ComputeShaderDesc,
        compute_shader_spirv: ComputeShaderDesc,
        resource_ranges: *const ResourceRangeDesc,
        resource_ranges_num: u32,
        has_constant_data: bool,
        shader_identifier: [c_char; 256],
    }

    #[repr(C)]
    struct DescriptorPoolDesc {
        per_set_textures_max_num: u32,
        per_set_storage_textures_max_num: u32,
        total_textures_num: u32,
        total_storage_textures_num: u32,
        sets_max_num: u32,
    }

    #[repr(C)]
    struct InstanceDesc {
        constant_buffer_and_samplers_space_index: u32,
        resources_space_index: u32,
        constant_buffer_register_index: u32,
        samplers_base_register_index: u32,
        resources_base_register_index: u32,
        constant_buffer_max_data_size: u32,
        samplers: *const u32,
        samplers_num: u32,
        shader_entry_point: *const c_char,
        pipelines: *const PipelineDesc,
        pipelines_num: u32,
        permanent_pool: *const TextureDesc,
        permanent_pool_size: u32,
        transient_pool: *const TextureDesc,
        transient_pool_size: u32,
        descriptor_pool_desc: DescriptorPoolDesc,
    }

    #[repr(C)]
    struct DispatchDesc {
        name: *const c_char,
        identifier: u32,
        resources: *const ResourceDesc,
        resources_num: u32,
        constant_buffer_data: *const u8,
        constant_buffer_data_size: u32,
        constant_buffer_data_matches_previous_dispatch: bool,
        pipeline_index: u16,
        grid_width: u16,
        grid_height: u16,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CommonSettings {
        view_to_clip_matrix: [f32; 16],
        view_to_clip_matrix_prev: [f32; 16],
        world_to_view_matrix: [f32; 16],
        world_to_view_matrix_prev: [f32; 16],
        world_prev_to_world_matrix: [f32; 16],
        motion_vector_scale: [f32; 3],
        motion_vector_bias: [f32; 3],
        camera_jitter: [f32; 2],
        camera_jitter_prev: [f32; 2],
        resource_size: [u16; 2],
        resource_size_prev: [u16; 2],
        rect_size: [u16; 2],
        rect_size_prev: [u16; 2],
        view_z_scale: f32,
        time_delta_between_frames: f32,
        denoising_range: f32,
        disocclusion_threshold: f32,
        disocclusion_threshold_alternate: f32,
        camera_attached_reflection_material_id: f32,
        strand_material_id: f32,
        history_fix_alternate_pixel_stride_material_id: f32,
        strand_thickness: f32,
        split_screen: f32,
        printf_at: [u16; 2],
        debug: f32,
        input_rect_origin: [u32; 2],
        output_rect_origin: [u32; 2],
        frame_index: u32,
        accumulation_mode: u8,
        is_motion_vector_in_world_space: bool,
        is_history_confidence_available: bool,
        is_disocclusion_threshold_mix_available: bool,
        enable_validation: bool,
    }

    #[repr(C)]
    struct SigmaSettings {
        light_direction: [f32; 3],
        plane_distance_sensitivity: f32,
        max_stabilized_frame_num: u32,
        checkerboard_mode: u8,
    }

    // Sizes and the offsets where padding decides, from MSVC's layout of NRD 4.18.0's headers.
    const _: () = {
        use std::mem::{offset_of, size_of};
        assert!(size_of::<AllocationCallbacks>() == 32);
        assert!(size_of::<LibraryDesc>() == 40);
        assert!(offset_of!(LibraryDesc, supported_denoisers) == 16);
        assert!(offset_of!(LibraryDesc, version_major) == 28);
        assert!(offset_of!(LibraryDesc, normal_encoding) == 31);
        assert!(offset_of!(LibraryDesc, roughness_encoding) == 32);
        assert!(size_of::<InstanceCreationDesc>() == 48);
        assert!(offset_of!(InstanceCreationDesc, denoisers_num) == 40);
        assert!(size_of::<TextureDesc>() == 8);
        assert!(size_of::<ResourceDesc>() == 12);
        assert!(size_of::<ResourceRangeDesc>() == 8);
        assert!(size_of::<PipelineDesc>() == 320);
        assert!(offset_of!(PipelineDesc, compute_shader_spirv) == 32);
        assert!(offset_of!(PipelineDesc, resource_ranges_num) == 56);
        assert!(offset_of!(PipelineDesc, has_constant_data) == 60);
        assert!(offset_of!(PipelineDesc, shader_identifier) == 61);
        assert!(size_of::<InstanceDesc>() == 112);
        assert!(offset_of!(InstanceDesc, samplers) == 24);
        assert!(offset_of!(InstanceDesc, shader_entry_point) == 40);
        assert!(offset_of!(InstanceDesc, pipelines_num) == 56);
        assert!(offset_of!(InstanceDesc, permanent_pool) == 64);
        assert!(offset_of!(InstanceDesc, transient_pool) == 80);
        assert!(offset_of!(InstanceDesc, descriptor_pool_desc) == 92);
        assert!(size_of::<DispatchDesc>() == 56);
        assert!(offset_of!(DispatchDesc, resources) == 16);
        assert!(offset_of!(DispatchDesc, constant_buffer_data) == 32);
        assert!(offset_of!(DispatchDesc, constant_buffer_data_matches_previous_dispatch) == 44);
        assert!(offset_of!(DispatchDesc, pipeline_index) == 46);
        assert!(offset_of!(DispatchDesc, grid_height) == 50);
        assert!(size_of::<CommonSettings>() == 452);
        assert!(offset_of!(CommonSettings, motion_vector_scale) == 320);
        assert!(offset_of!(CommonSettings, motion_vector_bias) == 332);
        assert!(offset_of!(CommonSettings, camera_jitter) == 344);
        assert!(offset_of!(CommonSettings, resource_size) == 360);
        assert!(offset_of!(CommonSettings, view_z_scale) == 376);
        assert!(offset_of!(CommonSettings, split_screen) == 412);
        assert!(offset_of!(CommonSettings, debug) == 420);
        assert!(offset_of!(CommonSettings, input_rect_origin) == 424);
        assert!(offset_of!(CommonSettings, output_rect_origin) == 432);
        assert!(offset_of!(CommonSettings, frame_index) == 440);
        assert!(offset_of!(CommonSettings, accumulation_mode) == 444);
        assert!(offset_of!(CommonSettings, enable_validation) == 448);
        assert!(size_of::<SigmaSettings>() == 24);
        assert!(offset_of!(SigmaSettings, checkerboard_mode) == 20);
    };

    /// `nrd::Result::SUCCESS`.
    const SUCCESS: u32 = 0;
    /// `nrd::Denoiser::SIGMA_SHADOW`.
    const DENOISER_SIGMA_SHADOW: u32 = 16;
    /// The identifier Forge gives its one denoiser.
    const IDENTIFIER: u32 = 0;
    /// `nrd::DescriptorType`.
    const DESCRIPTOR_TEXTURE: u32 = 0;
    const DESCRIPTOR_STORAGE_TEXTURE: u32 = 1;
    /// `nrd::ResourceType`, the ones SIGMA uses.
    const IN_MV: u32 = 0;
    const IN_NORMAL_ROUGHNESS: u32 = 1;
    const IN_VIEWZ: u32 = 2;
    const IN_PENUMBRA: u32 = 15;
    const OUT_SHADOW_TRANSLUCENCY: u32 = 27;
    const TRANSIENT_POOL: u32 = 30;
    const PERMANENT_POOL: u32 = 31;
    /// `nrd::Sampler`.
    const SAMPLER_NEAREST_CLAMP: u32 = 0;
    /// `nrd::NormalEncoding::RGBA16_SNORM` (also read from float formats) and
    /// `nrd::RoughnessEncoding::LINEAR`: what `tools/fetch-nrd.sh` builds.
    const NORMAL_ENCODING_RGBA16_SNORM: u8 = 4;
    const ROUGHNESS_ENCODING_LINEAR: u8 = 1;
    /// `nrd::AccumulationMode`.
    const ACCUMULATION_CONTINUE: u8 = 0;
    const ACCUMULATION_RESTART: u8 = 1;
    /// `CheckerboardMode::OFF`: every pixel holds its own samples.
    const CHECKERBOARD_OFF: u8 = 0;
    /// Dispatches a frame may hold (SIGMA has 6; its first frame adds a clear per image).
    const MAX_DISPATCHES: u64 = 64;

    /// `nrd::Format`, in its order, as Vulkan formats.
    const FORMATS: [vk::Format; 44] = [
        vk::Format::R8_UNORM,
        vk::Format::R8_SNORM,
        vk::Format::R8_UINT,
        vk::Format::R8_SINT,
        vk::Format::R8G8_UNORM,
        vk::Format::R8G8_SNORM,
        vk::Format::R8G8_UINT,
        vk::Format::R8G8_SINT,
        vk::Format::R8G8B8A8_UNORM,
        vk::Format::R8G8B8A8_SNORM,
        vk::Format::R8G8B8A8_UINT,
        vk::Format::R8G8B8A8_SINT,
        vk::Format::R8G8B8A8_SRGB,
        vk::Format::R16_UNORM,
        vk::Format::R16_SNORM,
        vk::Format::R16_UINT,
        vk::Format::R16_SINT,
        vk::Format::R16_SFLOAT,
        vk::Format::R16G16_UNORM,
        vk::Format::R16G16_SNORM,
        vk::Format::R16G16_UINT,
        vk::Format::R16G16_SINT,
        vk::Format::R16G16_SFLOAT,
        vk::Format::R16G16B16A16_UNORM,
        vk::Format::R16G16B16A16_SNORM,
        vk::Format::R16G16B16A16_UINT,
        vk::Format::R16G16B16A16_SINT,
        vk::Format::R16G16B16A16_SFLOAT,
        vk::Format::R32_UINT,
        vk::Format::R32_SINT,
        vk::Format::R32_SFLOAT,
        vk::Format::R32G32_UINT,
        vk::Format::R32G32_SINT,
        vk::Format::R32G32_SFLOAT,
        vk::Format::R32G32B32_UINT,
        vk::Format::R32G32B32_SINT,
        vk::Format::R32G32B32_SFLOAT,
        vk::Format::R32G32B32A32_UINT,
        vk::Format::R32G32B32A32_SINT,
        vk::Format::R32G32B32A32_SFLOAT,
        vk::Format::A2B10G10R10_UNORM_PACK32,
        vk::Format::A2B10G10R10_UINT_PACK32,
        vk::Format::B10G11R11_UFLOAT_PACK32,
        vk::Format::E5B9G9R9_UFLOAT_PACK32,
    ];

    type CreateInstance =
        unsafe extern "system" fn(*const InstanceCreationDesc, *mut *mut c_void) -> u32;
    type DestroyInstance = unsafe extern "system" fn(*mut c_void);
    type GetLibraryDesc = unsafe extern "system" fn() -> *const LibraryDesc;
    type GetInstanceDesc = unsafe extern "system" fn(*const c_void) -> *const InstanceDesc;
    type SetCommonSettings = unsafe extern "system" fn(*mut c_void, *const CommonSettings) -> u32;
    type SetDenoiserSettings = unsafe extern "system" fn(*mut c_void, u32, *const c_void) -> u32;
    type GetComputeDispatches = unsafe extern "system" fn(
        *mut c_void,
        *const u32,
        u32,
        *mut *const DispatchDesc,
        *mut u32,
    ) -> u32;

    fn check(call: &str, result: u32) -> Result<()> {
        const NAMES: [&str; 5] = [
            "SUCCESS",
            "FAILURE",
            "INVALID_ARGUMENT",
            "UNSUPPORTED",
            "NON_UNIQUE_IDENTIFIER",
        ];
        if result == SUCCESS {
            return Ok(());
        }
        let name = NAMES.get(result as usize).copied().unwrap_or("unknown");
        Err(GpuError::Nrd(format!("{call}: {name} ({result})")))
    }

    /// The library and one SIGMA instance in it.
    struct Library {
        instance: *mut c_void,
        destroy_instance: DestroyInstance,
        get_instance_desc: GetInstanceDesc,
        set_common_settings: SetCommonSettings,
        set_denoiser_settings: SetDenoiserSettings,
        get_compute_dispatches: GetComputeDispatches,
        /// The library's version (`4.18.0`).
        version: String,
        // Last: the functions above point into it.
        _library: libloading::Library,
    }

    // SAFETY: NRD's instance has no thread affinity; Forge calls it under `Sigma::state`'s
    // mutex only.
    unsafe impl Send for Library {}

    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: the instance was created by this library and is destroyed once.
            unsafe { (self.destroy_instance)(self.instance) };
        }
    }

    /// One pipeline of NRD's.
    struct NrdPipeline {
        raw: vk::Pipeline,
        layout: vk::PipelineLayout,
        set_layout: vk::DescriptorSetLayout,
        /// Per resource of a dispatch, in order: its binding and descriptor type.
        bindings: Vec<(u32, vk::DescriptorType)>,
    }

    pub(super) struct Sigma {
        device: Arc<Device>,
        extent: vk::Extent2D,
        state: Mutex<Library>,
        pipelines: Vec<NrdPipeline>,
        samplers: Vec<vk::Sampler>,
        shared_layout: vk::DescriptorSetLayout,
        pool: vk::DescriptorPool,
        shared_set: vk::DescriptorSet,
        /// Per frame slot, `MAX_DISPATCHES` constant blocks of `stride` bytes.
        constants: Buffer,
        stride: u64,
        permanent: Vec<GraphImage>,
        transient: Vec<GraphImage>,
        output: GraphImage,
        /// The graph's labels of NRD's dispatches (`shadow/<name>`), made once each.
        labels: Mutex<Vec<(String, &'static str)>>,
        /// NRD's clear pipelines: their dispatches become `vkCmdClearColorImage` (see the
        /// module's documentation).
        clear_pipelines: Vec<u16>,
    }

    impl Sigma {
        pub(super) fn load(
            device: &Arc<Device>,
            dir: &std::path::Path,
            extent: vk::Extent2D,
        ) -> Result<Self> {
            let features = device.features();
            if !features.push_descriptor
                || !features.storage_write_without_format
                || !features.pipeline_robustness
            {
                return Err(GpuError::Nrd(
                    "the device lacks push descriptors, unformatted storage writes or pipeline \
                     robustness"
                        .into(),
                ));
            }
            if extent.width > u32::from(u16::MAX) || extent.height > u32::from(u16::MAX) {
                return Err(GpuError::Nrd(format!("{extent:?} is too large")));
            }
            let path = dir.join(if cfg!(windows) {
                "NRD.dll"
            } else {
                "libNRD.so"
            });
            if !path.exists() {
                return Err(GpuError::Nrd(format!(
                    "{} not found (tools/fetch-nrd.sh builds it)",
                    path.display()
                )));
            }
            // SAFETY: loading NRD runs no initialisation code beyond the C++ runtime's.
            let library = unsafe { libloading::Library::new(&path) }.map_err(|error| {
                GpuError::Nrd(format!("cannot load {}: {error}", path.display()))
            })?;
            macro_rules! function {
                ($name:literal, $kind:ty) => {{
                    // SAFETY: the symbol is one of NRD's exported C functions (`NRD.h`), declared
                    // with its signature: references are pointers in the C ABI.
                    let symbol = unsafe { library.get::<$kind>($name) }.map_err(|error| {
                        GpuError::Nrd(format!("{}: {error}", String::from_utf8_lossy($name)))
                    })?;
                    *symbol
                }};
            }
            let get_library_desc = function!(b"GetLibraryDesc\0", GetLibraryDesc);
            let create_instance = function!(b"CreateInstance\0", CreateInstance);
            // SAFETY: the description is static in the library.
            let library_desc = unsafe { &*get_library_desc() };
            let version = format!(
                "{}.{}.{}",
                library_desc.version_major, library_desc.version_minor, library_desc.version_build
            );
            if format!("v{version}") != NRD_RELEASE {
                return Err(GpuError::Nrd(format!(
                    "{} is NRD {version}, Forge needs {NRD_RELEASE} (run tools/fetch-nrd.sh)",
                    path.display()
                )));
            }
            if library_desc.normal_encoding != NORMAL_ENCODING_RGBA16_SNORM
                || library_desc.roughness_encoding != ROUGHNESS_ENCODING_LINEAR
            {
                return Err(GpuError::Nrd(format!(
                    "built with normal encoding {} and roughness encoding {}; Forge writes {} and {} \
                     (run tools/fetch-nrd.sh)",
                    library_desc.normal_encoding,
                    library_desc.roughness_encoding,
                    NORMAL_ENCODING_RGBA16_SNORM,
                    ROUGHNESS_ENCODING_LINEAR
                )));
            }
            // SAFETY: the list is static in the library, `supported_denoisers_num` long.
            let supported = unsafe {
                std::slice::from_raw_parts(
                    library_desc.supported_denoisers,
                    library_desc.supported_denoisers_num as usize,
                )
            };
            if !supported.contains(&DENOISER_SIGMA_SHADOW) {
                return Err(GpuError::Nrd("this build has no SIGMA_SHADOW".into()));
            }
            let offsets = library_desc.spirv_binding_offsets;

            let denoisers = [DenoiserDesc {
                identifier: IDENTIFIER,
                denoiser: DENOISER_SIGMA_SHADOW,
            }];
            let creation = InstanceCreationDesc {
                // Null: NRD's own aligned allocator.
                allocation_callbacks: AllocationCallbacks {
                    allocate: std::ptr::null(),
                    reallocate: std::ptr::null(),
                    free: std::ptr::null(),
                    user_arg: std::ptr::null_mut(),
                },
                denoisers: denoisers.as_ptr(),
                denoisers_num: 1,
            };
            let mut instance = std::ptr::null_mut();
            // SAFETY: the description and the list it points to outlive the call.
            check("CreateInstance", unsafe {
                create_instance(&creation, &mut instance)
            })?;
            let state = Library {
                instance,
                destroy_instance: function!(b"DestroyInstance\0", DestroyInstance),
                get_instance_desc: function!(b"GetInstanceDesc\0", GetInstanceDesc),
                set_common_settings: function!(b"SetCommonSettings\0", SetCommonSettings),
                set_denoiser_settings: function!(b"SetDenoiserSettings\0", SetDenoiserSettings),
                get_compute_dispatches: function!(b"GetComputeDispatches\0", GetComputeDispatches),
                version,
                _library: library,
            };
            // SAFETY: the description lives as long as the instance.
            let desc = unsafe { &*(state.get_instance_desc)(state.instance) };
            Self::create(device, extent, state, desc, offsets)
        }

        fn create(
            device: &Arc<Device>,
            extent: vk::Extent2D,
            state: Library,
            desc: &InstanceDesc,
            offsets: SpirvBindingOffsets,
        ) -> Result<Self> {
            let raw = device.raw();
            // The samplers NRD asks for, immutable in the shared set.
            // SAFETY: the list is `samplers_num` long, owned by the instance.
            let kinds =
                unsafe { std::slice::from_raw_parts(desc.samplers, desc.samplers_num as usize) };
            let mut samplers = Vec::with_capacity(kinds.len());
            for &kind in kinds {
                let filter = if kind == SAMPLER_NEAREST_CLAMP {
                    vk::Filter::NEAREST
                } else {
                    vk::Filter::LINEAR
                };
                let info = vk::SamplerCreateInfo::default()
                    .mag_filter(filter)
                    .min_filter(filter)
                    .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .max_lod(vk::LOD_CLAMP_NONE);
                // SAFETY: valid create info.
                samplers.push(unsafe { raw.create_sampler(&info, None)? });
            }
            // Set 1 (NRD's constant buffer and samplers space): the samplers, then the constants
            // at a dynamic offset into the ring.
            let mut shared_bindings: Vec<vk::DescriptorSetLayoutBinding<'_>> = samplers
                .iter()
                .enumerate()
                .map(|(i, sampler)| {
                    vk::DescriptorSetLayoutBinding::default()
                        .binding(offsets.sampler + desc.samplers_base_register_index + i as u32)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .descriptor_count(1)
                        .stage_flags(vk::ShaderStageFlags::COMPUTE)
                        .immutable_samplers(std::slice::from_ref(sampler))
                })
                .collect();
            let constants_binding = offsets.constant_buffer + desc.constant_buffer_register_index;
            shared_bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(constants_binding)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::COMPUTE),
            );
            let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&shared_bindings);
            // SAFETY: valid create info; the samplers outlive the layout.
            let shared_layout = unsafe { raw.create_descriptor_set_layout(&info, None)? };
            if desc.constant_buffer_and_samplers_space_index != 1 || desc.resources_space_index != 0
            {
                return Err(GpuError::Nrd(format!(
                    "register spaces {} (constants) and {} (resources), Forge maps 1 and 0",
                    desc.constant_buffer_and_samplers_space_index, desc.resources_space_index
                )));
            }

            // SAFETY: the entry point is a static string of the library.
            let entry = unsafe { CStr::from_ptr(desc.shader_entry_point) }.to_owned();
            // SAFETY: the list is `pipelines_num` long, owned by the instance.
            let descs =
                unsafe { std::slice::from_raw_parts(desc.pipelines, desc.pipelines_num as usize) };
            let mut pipelines = Vec::with_capacity(descs.len());
            let mut clear_pipelines = Vec::new();
            for (index, p) in descs.iter().enumerate() {
                if p.compute_shader_spirv.bytecode.is_null() {
                    return Err(GpuError::Nrd("the library holds no SPIR-V".into()));
                }
                // SAFETY: the bytecode is `size` bytes owned by the library.
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        p.compute_shader_spirv.bytecode.cast::<u8>(),
                        p.compute_shader_spirv.size as usize,
                    )
                };
                let words: Vec<u32> = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|&w| u32::from_le_bytes(w))
                    .collect();
                // SAFETY: the list is `resource_ranges_num` long, owned by the instance.
                let ranges = unsafe {
                    std::slice::from_raw_parts(p.resource_ranges, p.resource_ranges_num as usize)
                };
                let mut bindings = Vec::new();
                for range in ranges {
                    let (base, kind) = match range.descriptor_type {
                        DESCRIPTOR_TEXTURE => (offsets.texture, vk::DescriptorType::SAMPLED_IMAGE),
                        DESCRIPTOR_STORAGE_TEXTURE => (
                            offsets.storage_texture_and_buffer,
                            vk::DescriptorType::STORAGE_IMAGE,
                        ),
                        other => {
                            return Err(GpuError::Nrd(format!("descriptor type {other}")));
                        }
                    };
                    for k in 0..range.descriptors_num {
                        bindings.push((base + desc.resources_base_register_index + k, kind));
                    }
                }
                let layout_bindings: Vec<_> = bindings
                    .iter()
                    .map(|&(binding, kind)| {
                        vk::DescriptorSetLayoutBinding::default()
                            .binding(binding)
                            .descriptor_type(kind)
                            .descriptor_count(1)
                            .stage_flags(vk::ShaderStageFlags::COMPUTE)
                    })
                    .collect();
                let info = vk::DescriptorSetLayoutCreateInfo::default()
                    .flags(vk::DescriptorSetLayoutCreateFlags::PUSH_DESCRIPTOR_KHR)
                    .bindings(&layout_bindings);
                // SAFETY: valid create info on a device with push descriptors.
                let set_layout = unsafe { raw.create_descriptor_set_layout(&info, None)? };
                let set_layouts = [set_layout, shared_layout];
                let info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
                // SAFETY: valid create info; the set layouts are alive.
                let layout = unsafe { raw.create_pipeline_layout(&info, None)? };
                // SAFETY: NRD's SPIR-V, compiled by its build with DXC for Vulkan 1.2.
                let module = device.create_shader_module(&words, "nrd")?;
                let stage = vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::COMPUTE)
                    .module(module)
                    .name(&entry);
                // D3D12's behaviour outside an image, for NRD's kernels only (module docs).
                let mut robust = vk::PipelineRobustnessCreateInfoEXT::default()
                    .storage_buffers(vk::PipelineRobustnessBufferBehaviorEXT::DEVICE_DEFAULT)
                    .uniform_buffers(vk::PipelineRobustnessBufferBehaviorEXT::DEVICE_DEFAULT)
                    .vertex_inputs(vk::PipelineRobustnessBufferBehaviorEXT::DEVICE_DEFAULT)
                    .images(vk::PipelineRobustnessImageBehaviorEXT::ROBUST_IMAGE_ACCESS);
                let info = vk::ComputePipelineCreateInfo::default()
                    .stage(stage)
                    .layout(layout)
                    .push_next(&mut robust);
                // SAFETY: valid create info; the module outlives the call.
                let created = unsafe {
                    raw.create_compute_pipelines(vk::PipelineCache::null(), &[info], None)
                };
                device.destroy_shader_module(module);
                let pipeline = created.map_err(|(_, e)| e)?[0];
                // SAFETY: the identifier is a NUL-terminated string in the description.
                let name = unsafe { CStr::from_ptr(p.shader_identifier.as_ptr()) };
                if name.to_bytes().starts_with(b"Clear.cs") {
                    clear_pipelines.push(index as u16);
                }
                device.set_name(
                    pipeline,
                    &format!("nrd {index}: {}", name.to_string_lossy()),
                );
                pipelines.push(NrdPipeline {
                    raw: pipeline,
                    layout,
                    set_layout,
                    bindings,
                });
            }

            // The ring of constants: per frame slot, a block per dispatch.
            let alignment = device.limits().min_uniform_buffer_offset_alignment.max(16);
            let block = u64::from(desc.constant_buffer_max_data_size.max(16));
            let stride = block.div_ceil(alignment) * alignment;
            let constants = device.create_buffer(BufferDesc {
                size: stride * MAX_DISPATCHES * FRAMES_IN_FLIGHT as u64,
                usage: vk::BufferUsageFlags::UNIFORM_BUFFER,
                location: gpu_allocator::MemoryLocation::CpuToGpu,
                category: MemoryCategory::Frame,
                name: "nrd constants",
            })?;
            let sizes = [
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::SAMPLER,
                    descriptor_count: samplers.len().max(1) as u32,
                },
                vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
                    descriptor_count: 1,
                },
            ];
            let info = vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&sizes);
            // SAFETY: valid create info.
            let pool = unsafe { raw.create_descriptor_pool(&info, None)? };
            let layouts = [shared_layout];
            let info = vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(&layouts);
            // SAFETY: the pool has room for this one set.
            let shared_set = unsafe { raw.allocate_descriptor_sets(&info)? }[0];
            let buffer_info = [vk::DescriptorBufferInfo {
                buffer: constants.raw(),
                offset: 0,
                range: block,
            }];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(shared_set)
                .dst_binding(constants_binding)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .buffer_info(&buffer_info);
            // SAFETY: the set and the buffer are alive; nothing uses the set yet.
            unsafe { raw.update_descriptor_sets(&[write], &[]) };

            // NRD's pools: persistent images. The transient pool could alias with the frame's
            // transients, but NRD leaves some texels unwritten (the sky's), whose contents would
            // then depend on what the memory held before.
            let image = |pool: &str, index: usize, t: &TextureDesc| -> Result<GraphImage> {
                let format = *FORMATS
                    .get(t.format as usize)
                    .ok_or_else(|| GpuError::Nrd(format!("format {}", t.format)))?;
                let factor = u32::from(t.downsample_factor.max(1));
                GraphImage::new(
                    device,
                    ImageDesc {
                        width: extent.width.div_ceil(factor),
                        height: extent.height.div_ceil(factor),
                        format,
                        usage: vk::ImageUsageFlags::SAMPLED
                            | vk::ImageUsageFlags::STORAGE
                            | vk::ImageUsageFlags::TRANSFER_DST,
                        aspect: vk::ImageAspectFlags::COLOR,
                        mip_levels: 1,
                        name: &format!("nrd {pool} {index}"),
                    },
                )
            };
            // SAFETY: the pools are `*_size` long, owned by the instance.
            let (permanent_descs, transient_descs) = unsafe {
                (
                    std::slice::from_raw_parts(
                        desc.permanent_pool,
                        desc.permanent_pool_size as usize,
                    ),
                    std::slice::from_raw_parts(
                        desc.transient_pool,
                        desc.transient_pool_size as usize,
                    ),
                )
            };
            let permanent = permanent_descs
                .iter()
                .enumerate()
                .map(|(i, t)| image("permanent", i, t))
                .collect::<Result<Vec<_>>>()?;
            let transient = transient_descs
                .iter()
                .enumerate()
                .map(|(i, t)| image("transient", i, t))
                .collect::<Result<Vec<_>>>()?;
            let output = GraphImage::new(
                device,
                ImageDesc {
                    width: extent.width,
                    height: extent.height,
                    format: SHADOW_FORMAT,
                    usage: vk::ImageUsageFlags::SAMPLED
                        | vk::ImageUsageFlags::STORAGE
                        | vk::ImageUsageFlags::TRANSFER_DST,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name: "sun shadow (SIGMA)",
                },
            )?;
            tracing::info!(
                version = %state.version,
                pipelines = pipelines.len(),
                permanent = permanent.len(),
                transient = transient.len(),
                ?extent,
                "NRD SIGMA loaded"
            );
            Ok(Self {
                device: Arc::clone(device),
                extent,
                state: Mutex::new(state),
                pipelines,
                samplers,
                shared_layout,
                pool,
                shared_set,
                constants,
                stride,
                permanent,
                transient,
                output,
                labels: Mutex::new(Vec::new()),
                clear_pipelines,
            })
        }

        pub(super) fn extent(&self) -> vk::Extent2D {
            self.extent
        }

        pub(super) fn version(&self) -> String {
            self.state.lock().version.clone()
        }

        /// The graph's label for a dispatch NRD calls `name`.
        fn label(&self, name: *const c_char) -> &'static str {
            let name = if name.is_null() {
                "SIGMA".to_owned()
            } else {
                // SAFETY: NRD's dispatch names are static NUL-terminated strings.
                unsafe { CStr::from_ptr(name) }
                    .to_string_lossy()
                    .into_owned()
            };
            let mut labels = self.labels.lock();
            if let Some((_, label)) = labels.iter().find(|(n, _)| *n == name) {
                return label;
            }
            // A handful of names over the program's life: leaked once each, as the graph's
            // labels are static. "SIGMA_Shadow - Blur" shows as "shadow/SIGMA blur".
            let short = name.strip_prefix("SIGMA_Shadow - ").unwrap_or(&name);
            let label: &'static str =
                Box::leak(format!("shadow/SIGMA {}", short.to_lowercase()).into_boxed_str());
            labels.push((name, label));
            label
        }

        pub(super) fn denoise<'f>(
            &'f self,
            graph: &mut FrameGraph<'f>,
            slot: FrameSlot,
            images: SigmaImages,
            frame: &SigmaFrame,
        ) -> Result<ImageHandle> {
            let size = [self.extent.width as u16, self.extent.height as u16];
            let settings = CommonSettings {
                view_to_clip_matrix: frame.view_to_clip,
                view_to_clip_matrix_prev: frame.view_to_clip_prev,
                world_to_view_matrix: frame.world_to_view,
                world_to_view_matrix_prev: frame.world_to_view_prev,
                world_prev_to_world_matrix: [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                // TAA's motion is already the UV offset to the previous frame.
                motion_vector_scale: [1.0, 1.0, 0.0],
                motion_vector_bias: [0.0; 3],
                camera_jitter: frame.jitter,
                camera_jitter_prev: frame.jitter_prev,
                resource_size: size,
                resource_size_prev: size,
                rect_size: size,
                rect_size_prev: size,
                view_z_scale: 1.0,
                // Always given: NRD would otherwise time the frames itself, and captures would
                // depend on the machine's speed.
                time_delta_between_frames: frame.time_delta_ms.max(0.001),
                denoising_range: frame.denoising_range,
                disocclusion_threshold: 0.01,
                disocclusion_threshold_alternate: 0.05,
                camera_attached_reflection_material_id: 999.0,
                strand_material_id: 999.0,
                history_fix_alternate_pixel_stride_material_id: 999.0,
                strand_thickness: 80e-6,
                split_screen: 0.0,
                printf_at: [9999, 9999],
                debug: 0.0,
                input_rect_origin: [0, 0],
                output_rect_origin: [0, 0],
                frame_index: frame.frame_index,
                accumulation_mode: if frame.reset {
                    ACCUMULATION_RESTART
                } else {
                    ACCUMULATION_CONTINUE
                },
                is_motion_vector_in_world_space: false,
                is_history_confidence_available: false,
                is_disocclusion_threshold_mix_available: false,
                enable_validation: false,
            };
            let sigma = SigmaSettings {
                light_direction: frame.light_direction,
                plane_distance_sensitivity: 0.02,
                max_stabilized_frame_num: frame.stabilized_frames.min(7),
                checkerboard_mode: CHECKERBOARD_OFF,
            };
            let state = self.state.lock();
            let mut dispatches: *const DispatchDesc = std::ptr::null();
            let mut count = 0u32;
            // SAFETY: the settings outlive the calls, which copy them; the dispatch list is owned
            // by the instance until the next call, and read below under the same lock.
            unsafe {
                check(
                    "SetCommonSettings",
                    (state.set_common_settings)(state.instance, &settings),
                )?;
                check(
                    "SetDenoiserSettings",
                    (state.set_denoiser_settings)(
                        state.instance,
                        IDENTIFIER,
                        (&raw const sigma).cast(),
                    ),
                )?;
                check(
                    "GetComputeDispatches",
                    (state.get_compute_dispatches)(
                        state.instance,
                        &IDENTIFIER,
                        1,
                        &mut dispatches,
                        &mut count,
                    ),
                )?;
            }
            if u64::from(count) > MAX_DISPATCHES {
                return Err(GpuError::Nrd(format!("{count} dispatches in a frame")));
            }
            // SAFETY: as above, `count` long.
            let dispatches = unsafe { std::slice::from_raw_parts(dispatches, count as usize) };

            let permanent: Vec<ImageHandle> =
                self.permanent.iter().map(|i| graph.import(i)).collect();
            let transient: Vec<ImageHandle> =
                self.transient.iter().map(|i| graph.import(i)).collect();
            let output = graph.import(&self.output);
            let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
            for (index, dispatch) in dispatches.iter().enumerate() {
                let pipeline = self
                    .pipelines
                    .get(usize::from(dispatch.pipeline_index))
                    .ok_or_else(|| {
                        GpuError::Nrd(format!("pipeline {}", dispatch.pipeline_index))
                    })?;
                // SAFETY: the list is `resources_num` long, owned by the instance.
                let resources = unsafe {
                    std::slice::from_raw_parts(dispatch.resources, dispatch.resources_num as usize)
                };
                if resources.len() != pipeline.bindings.len() {
                    return Err(GpuError::Nrd(format!(
                        "a dispatch of {} resources for a pipeline of {}",
                        resources.len(),
                        pipeline.bindings.len()
                    )));
                }
                let mut bound = Vec::with_capacity(resources.len());
                for r in resources {
                    let pool = |list: &[ImageHandle]| {
                        list.get(usize::from(r.index_in_pool))
                            .copied()
                            .ok_or_else(|| {
                                GpuError::Nrd(format!(
                                    "pool image {} of type {}",
                                    r.index_in_pool, r.resource_type
                                ))
                            })
                    };
                    let handle = match r.resource_type {
                        IN_MV => images.motion,
                        IN_NORMAL_ROUGHNESS => images.normal_roughness,
                        IN_VIEWZ => images.view_z,
                        IN_PENUMBRA => images.penumbra,
                        OUT_SHADOW_TRANSLUCENCY => output,
                        TRANSIENT_POOL => pool(&transient)?,
                        PERMANENT_POOL => pool(&permanent)?,
                        other => return Err(GpuError::Nrd(format!("resource type {other}"))),
                    };
                    bound.push((handle, r.descriptor_type == DESCRIPTOR_STORAGE_TEXTURE));
                }
                // NRD's clears (its first frame): the whole image cleared to zero by Vulkan.
                if self.clear_pipelines.contains(&dispatch.pipeline_index) {
                    let [(image, true)] = bound[..] else {
                        return Err(GpuError::Nrd("a clear of other than one image".into()));
                    };
                    graph
                        .pass(self.label(dispatch.name))
                        .image(image, ImageAccess::TransferDst)
                        .run(move |resources, commands| {
                            let range = vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .level_count(1)
                                .layer_count(1);
                            // SAFETY: recording state; the pass declared the image as a transfer
                            // destination, so the graph put it in that layout.
                            unsafe {
                                self.device.raw().cmd_clear_color_image(
                                    commands.raw(),
                                    resources.image(image).raw,
                                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                                    &vk::ClearColorValue { uint32: [0; 4] },
                                    &[range],
                                );
                            }
                            Ok(())
                        });
                    continue;
                }
                // The dispatch's constants, copied now: NRD overwrites the list next frame.
                let offset = (slot.index as u64 * MAX_DISPATCHES + index as u64) * self.stride;
                if dispatch.constant_buffer_data_size > 0 {
                    // SAFETY: `constant_buffer_data_size` bytes owned by the instance.
                    let data = unsafe {
                        std::slice::from_raw_parts(
                            dispatch.constant_buffer_data,
                            dispatch.constant_buffer_data_size as usize,
                        )
                    };
                    self.constants.write(offset, data);
                }
                // Each image once: read (sampled), written (storage), or both.
                let mut builder = graph.pass(self.label(dispatch.name));
                let mut declared: Vec<(ImageHandle, bool, bool)> = Vec::new();
                for &(handle, storage) in &bound {
                    match declared.iter_mut().find(|(h, ..)| *h == handle) {
                        Some((_, read, write)) => {
                            *read |= !storage;
                            *write |= storage;
                        }
                        None => declared.push((handle, !storage, storage)),
                    }
                }
                for &(handle, read, write) in &declared {
                    let access = match (read, write) {
                        (true, false) => ImageAccess::Sampled(compute),
                        (false, _) => ImageAccess::StorageWrite(compute),
                        (true, true) => ImageAccess::Custom {
                            layout: vk::ImageLayout::GENERAL,
                            stages: compute,
                            access: vk::AccessFlags2::SHADER_SAMPLED_READ
                                | vk::AccessFlags2::SHADER_STORAGE_WRITE,
                        },
                    };
                    builder = builder.image(handle, access);
                }
                let grid = (
                    u32::from(dispatch.grid_width),
                    u32::from(dispatch.grid_height),
                );
                let dynamic_offset = u32::try_from(offset)
                    .map_err(|_| GpuError::Nrd("the constants' ring is too large".into()))?;
                builder.run(move |resources, commands| {
                    let raw = self.device.raw();
                    let push = self
                        .device
                        .push_descriptor_loader()
                        .expect("checked when NRD loaded");
                    let infos: Vec<[vk::DescriptorImageInfo; 1]> = bound
                        .iter()
                        .map(|&(handle, storage)| {
                            let image = resources.image(handle);
                            let layout = if storage
                                || declared.iter().any(|&(h, r, w)| h == handle && r && w)
                            {
                                vk::ImageLayout::GENERAL
                            } else {
                                image.sampled_layout
                            };
                            [vk::DescriptorImageInfo::default()
                                .image_view(image.view)
                                .image_layout(layout)]
                        })
                        .collect();
                    let writes: Vec<vk::WriteDescriptorSet<'_>> = pipeline
                        .bindings
                        .iter()
                        .zip(&infos)
                        .map(|(&(binding, kind), info)| {
                            vk::WriteDescriptorSet::default()
                                .dst_binding(binding)
                                .descriptor_type(kind)
                                .image_info(info)
                        })
                        .collect();
                    let cb = commands.raw();
                    // SAFETY: recording state; the pipeline, its layout and the shared set live
                    // as long as `self`; every pushed image was declared by this pass, so the
                    // graph put it in the layout written here.
                    unsafe {
                        raw.cmd_bind_pipeline(cb, vk::PipelineBindPoint::COMPUTE, pipeline.raw);
                        raw.cmd_bind_descriptor_sets(
                            cb,
                            vk::PipelineBindPoint::COMPUTE,
                            pipeline.layout,
                            1,
                            &[self.shared_set],
                            &[dynamic_offset],
                        );
                        push.cmd_push_descriptor_set(
                            cb,
                            vk::PipelineBindPoint::COMPUTE,
                            pipeline.layout,
                            0,
                            &writes,
                        );
                    }
                    commands.dispatch(grid.0, grid.1, 1);
                    Ok(())
                });
            }
            Ok(output)
        }
    }

    impl Drop for Sigma {
        fn drop(&mut self) {
            let raw = self.device.raw();
            // SAFETY: the owner made sure the GPU is done with the frames that used these (the
            // rule of every resource here); each handle is destroyed once.
            unsafe {
                for p in &self.pipelines {
                    raw.destroy_pipeline(p.raw, None);
                    raw.destroy_pipeline_layout(p.layout, None);
                    raw.destroy_descriptor_set_layout(p.set_layout, None);
                }
                raw.destroy_descriptor_pool(self.pool, None);
                raw.destroy_descriptor_set_layout(self.shared_layout, None);
                for &sampler in &self.samplers {
                    raw.destroy_sampler(sampler, None);
                }
            }
        }
    }
}

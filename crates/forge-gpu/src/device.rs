use std::ffi::CStr;
use std::sync::Arc;

use ash::{ext, khr, vk};
use gpu_allocator::vulkan::{Allocator, AllocatorCreateDesc};
use parking_lot::Mutex;

use crate::bindless::{Bindless, SampledImageId, StorageImageId};
use crate::error::{GpuError, Result};
use crate::instance::Instance;

/// Optional capabilities detected on the device.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeviceFeatures {
    /// `VK_EXT_mesh_shader` with both task and mesh stages.
    pub mesh_shader: bool,
    /// `VK_KHR_ray_query`, acceleration structures and the ray-tracing pipeline extension.
    pub ray_query: bool,
    /// `VK_EXT_sampler_filter_minmax` (min-reduction sampling for depth pyramids).
    pub sampler_minmax: bool,
    /// `VK_EXT_memory_budget` (per-heap usage and budget from the OS).
    pub memory_budget: bool,
    /// 8-bit index buffers (`VK_KHR_index_type_uint8` or the EXT): the meshlet fallback draws
    /// straight from the cooked one-byte triangle lists.
    pub index_type_uint8: bool,
    /// 64-bit atomics on storage buffers (`shaderBufferInt64Atomics`), usable from fragment
    /// shaders (`fragmentStoresAndAtomics`): the meshlet renderer's visibility buffer.
    pub int64_atomics: bool,
}

/// Choices made when creating a device.
#[derive(Clone, Copy, Debug, Default)]
pub struct DeviceOptions {
    /// Leave `VK_EXT_mesh_shader` disabled even when the GPU has it, so the renderers take
    /// their fallback paths on this GPU (`--force-fallback` in the demos).
    pub no_mesh_shader: bool,
}

/// Mesh-shader limits worth knowing when sizing meshlets.
#[derive(Clone, Copy, Debug, Default)]
pub struct MeshShaderLimits {
    /// Preferred task workgroup size (32 on NVIDIA).
    pub preferred_task_invocations: u32,
    /// Preferred mesh workgroup size (32 on NVIDIA).
    pub preferred_mesh_invocations: u32,
    /// Maximum vertices a mesh workgroup may emit.
    pub max_output_vertices: u32,
    /// Maximum primitives a mesh workgroup may emit.
    pub max_output_primitives: u32,
    /// Most mesh workgroups one draw may launch (`maxMeshWorkGroupTotalCount`: 4 194 304 on
    /// NVIDIA and on AMD's RADV).
    pub max_total_work_groups: u32,
}

/// A logical device with its graphics queue, extension loaders and memory allocator.
pub struct Device {
    instance: Arc<Instance>,
    physical: vk::PhysicalDevice,
    raw: ash::Device,
    graphics_family: u32,
    graphics_queue: vk::Queue,
    features: DeviceFeatures,
    mesh_limits: Option<MeshShaderLimits>,
    timestamp_period_ns: f32,
    limits: vk::PhysicalDeviceLimits,
    name: String,
    swapchain_loader: khr::swapchain::Device,
    mesh_loader: Option<ext::mesh_shader::Device>,
    /// Acceleration structures, with ray queries.
    acceleration_loader: Option<khr::acceleration_structure::Device>,
    /// `minAccelerationStructureScratchOffsetAlignment` (256 on the RTX 5070 Ti and the
    /// RX 9070 XT), with ray queries; 1 without.
    scratch_alignment: u64,
    debug_utils: Option<ext::debug_utils::Device>,
    allocator: Mutex<Option<Allocator>>,
    bindless: Mutex<Option<Bindless>>,
    bindless_layout: vk::DescriptorSetLayout,
    bindless_set: vk::DescriptorSet,
    /// Bytes allocated per category, uploaded and read back.
    memory_counters: crate::memory_report::MemoryCounters,
    /// `FORGE_VRAM_BUDGET_MB`: a smaller device-local budget than the OS gives (testing the
    /// warning, rehearsing a smaller card).
    budget_cap: Option<u64>,
    /// DLSS, when the instance came through Streamline and this GPU runs it.
    dlss: Option<crate::dlss::Dlss>,
}

struct Candidate {
    physical: vk::PhysicalDevice,
    graphics_family: u32,
    features: DeviceFeatures,
    /// The uint8 index extension to enable (the KHR name when offered, else the EXT one).
    index_type_uint8: Option<&'static CStr>,
    score: u32,
    name: String,
    /// PCI vendor id ([`VENDOR_NVIDIA`], AMD 0x1002, …).
    vendor_id: u32,
}

/// NVIDIA's PCI vendor id: the only vendor NVIDIA Streamline (DLSS) is loaded for (issue #67).
pub const VENDOR_NVIDIA: u32 = 0x10DE;

impl Device {
    /// Picks the best physical device that can present to `surface` (when given) and creates
    /// the logical device with the engine's baseline features plus the optional ones found.
    pub fn new(instance: Arc<Instance>, surface: Option<vk::SurfaceKHR>) -> Result<Arc<Self>> {
        Self::with_options(instance, surface, DeviceOptions::default())
    }

    /// [`Device::new`] with explicit [`DeviceOptions`].
    pub fn with_options(
        instance: Arc<Instance>,
        surface: Option<vk::SurfaceKHR>,
        options: DeviceOptions,
    ) -> Result<Arc<Self>> {
        let raw_instance = instance.raw();
        let mut best = Self::best_candidate(&instance, surface)?.ok_or_else(|| {
            GpuError::Unsupported("no Vulkan 1.3 device with a graphics queue".into())
        })?;
        if options.no_mesh_shader && best.features.mesh_shader {
            tracing::info!("mesh shaders left disabled (fallback paths forced)");
            best.features.mesh_shader = false;
        }
        if best.features.ray_query
            && std::env::var_os("FORGE_NO_RAY_QUERY").is_some_and(|v| v != "0")
        {
            tracing::info!("ray queries left disabled (FORGE_NO_RAY_QUERY)");
            best.features.ray_query = false;
        }
        tracing::info!(device = %best.name, features = ?best.features, "selected GPU");

        let mut extensions: Vec<*const i8> = Vec::new();
        if surface.is_some() {
            extensions.push(khr::swapchain::NAME.as_ptr());
        }
        if best.features.mesh_shader {
            extensions.push(ext::mesh_shader::NAME.as_ptr());
        }
        if best.features.sampler_minmax {
            extensions.push(ext::sampler_filter_minmax::NAME.as_ptr());
        }
        if best.features.memory_budget {
            extensions.push(ext::memory_budget::NAME.as_ptr());
        }
        if let Some(name) = best.index_type_uint8 {
            extensions.push(name.as_ptr());
        }
        if best.features.ray_query {
            extensions.push(khr::acceleration_structure::NAME.as_ptr());
            extensions.push(khr::ray_query::NAME.as_ptr());
            extensions.push(khr::deferred_host_operations::NAME.as_ptr());
            extensions.push(khr::ray_tracing_pipeline::NAME.as_ptr());
        }

        let base = vk::PhysicalDeviceFeatures::default()
            .shader_int64(true)
            .shader_int16(true)
            .sampler_anisotropy(true)
            .multi_draw_indirect(true)
            // The meshlet fallback's indirect draws carry the visible-list slot in firstInstance.
            .draw_indirect_first_instance(true)
            .fill_mode_non_solid(true)
            .fragment_stores_and_atomics(best.features.int64_atomics)
            // No geometry shaders are ever used (D-003), but a fragment shader that reads
            // `SV_PrimitiveID` (the visibility buffer's) declares the SPIR-V `Geometry`
            // capability, which the validation layer ties to this feature.
            .geometry_shader(true);
        let mut v11 = vk::PhysicalDeviceVulkan11Features::default().shader_draw_parameters(true);
        let mut v12 = vk::PhysicalDeviceVulkan12Features::default()
            .timeline_semaphore(true)
            .buffer_device_address(true)
            .descriptor_indexing(true)
            .runtime_descriptor_array(true)
            .shader_sampled_image_array_non_uniform_indexing(true)
            .shader_storage_buffer_array_non_uniform_indexing(true)
            .shader_storage_image_array_non_uniform_indexing(true)
            .descriptor_binding_partially_bound(true)
            .descriptor_binding_sampled_image_update_after_bind(true)
            .descriptor_binding_storage_image_update_after_bind(true)
            .descriptor_binding_storage_buffer_update_after_bind(true)
            .descriptor_binding_update_unused_while_pending(true)
            .descriptor_binding_variable_descriptor_count(true)
            .sampler_filter_minmax(best.features.sampler_minmax)
            .scalar_block_layout(true)
            .host_query_reset(true)
            .draw_indirect_count(true)
            .shader_buffer_int64_atomics(best.features.int64_atomics)
            .shader_float16(true)
            .shader_int8(true)
            .storage_buffer8_bit_access(true)
            .uniform_and_storage_buffer8_bit_access(true);
        let mut v13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true)
            .maintenance4(true)
            .shader_demote_to_helper_invocation(true)
            .subgroup_size_control(true)
            // Required of every Vulkan 1.3 device; Streamline (DLSS) creates private data slots.
            .private_data(true);
        // Mesh shaders only: no pass has a task stage.
        let mut mesh = vk::PhysicalDeviceMeshShaderFeaturesEXT::default().mesh_shader(true);
        let mut features2 = vk::PhysicalDeviceFeatures2::default()
            .features(base)
            .push_next(&mut v11)
            .push_next(&mut v12)
            .push_next(&mut v13);
        if best.features.mesh_shader {
            features2 = features2.push_next(&mut mesh);
        }
        let mut uint8 =
            vk::PhysicalDeviceIndexTypeUint8FeaturesKHR::default().index_type_uint8(true);
        if best.features.index_type_uint8 {
            features2 = features2.push_next(&mut uint8);
        }
        let mut acceleration = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
            .acceleration_structure(true);
        let mut ray_query = vk::PhysicalDeviceRayQueryFeaturesKHR::default().ray_query(true);
        let mut ray_tracing =
            vk::PhysicalDeviceRayTracingPipelineFeaturesKHR::default().ray_tracing_pipeline(true);
        if best.features.ray_query {
            features2 = features2
                .push_next(&mut acceleration)
                .push_next(&mut ray_query)
                .push_next(&mut ray_tracing);
        }
        let priorities = [1.0_f32];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(best.graphics_family)
            .queue_priorities(&priorities)];
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&extensions)
            .push_next(&mut features2);
        // SAFETY: `info` and everything it points to live until the call returns.
        let raw = unsafe { raw_instance.create_device(best.physical, &info, None)? };
        // SAFETY: the queue was requested in `queue_info`.
        let graphics_queue = unsafe { raw.get_device_queue(best.graphics_family, 0) };

        let mut mesh_props = vk::PhysicalDeviceMeshShaderPropertiesEXT::default();
        let mut acceleration_props =
            vk::PhysicalDeviceAccelerationStructurePropertiesKHR::default();
        let mut props2 = vk::PhysicalDeviceProperties2::default();
        if best.features.mesh_shader {
            props2 = props2.push_next(&mut mesh_props);
        }
        if best.features.ray_query {
            props2 = props2.push_next(&mut acceleration_props);
        }
        // SAFETY: property query on a valid physical device.
        unsafe { raw_instance.get_physical_device_properties2(best.physical, &mut props2) };
        let timestamp_period_ns = props2.properties.limits.timestamp_period;
        let limits = props2.properties.limits;
        let max_anisotropy = props2.properties.limits.max_sampler_anisotropy;
        let scratch_alignment = u64::from(
            acceleration_props
                .min_acceleration_structure_scratch_offset_alignment
                .max(1),
        );
        let mesh_limits = best.features.mesh_shader.then_some(MeshShaderLimits {
            preferred_task_invocations: mesh_props.max_preferred_task_work_group_invocations,
            preferred_mesh_invocations: mesh_props.max_preferred_mesh_work_group_invocations,
            max_output_vertices: mesh_props.max_mesh_output_vertices,
            max_output_primitives: mesh_props.max_mesh_output_primitives,
            max_total_work_groups: mesh_props.max_mesh_work_group_total_count,
        });

        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: raw_instance.clone(),
            device: raw.clone(),
            physical_device: best.physical,
            debug_settings: Default::default(),
            buffer_device_address: true,
            allocation_sizes: Default::default(),
        })?;

        let swapchain_loader = khr::swapchain::Device::new(raw_instance, &raw);
        let acceleration_loader = best
            .features
            .ray_query
            .then(|| khr::acceleration_structure::Device::new(raw_instance, &raw));
        let mesh_loader = best
            .features
            .mesh_shader
            .then(|| ext::mesh_shader::Device::new(raw_instance, &raw));
        let debug_utils = instance
            .validation_enabled()
            .then(|| ext::debug_utils::Device::new(raw_instance, &raw));
        let bindless = Bindless::new(&raw, max_anisotropy, best.features.sampler_minmax)?;
        let bindless_layout = bindless.layout();
        let bindless_set = bindless.set();
        #[cfg(all(feature = "dlss", windows))]
        let dlss = instance.streamline().and_then(|streamline| {
            match streamline.dlss_supported(vk::Handle::as_raw(best.physical)) {
                Ok(()) => {
                    tracing::info!("DLSS is available");
                    Some(crate::dlss::Dlss::new(Arc::clone(streamline)))
                }
                Err(error) => {
                    tracing::warn!(%error, "DLSS is not available on this GPU");
                    None
                }
            }
        });
        #[cfg(not(all(feature = "dlss", windows)))]
        let dlss = None;

        Ok(Arc::new(Self {
            instance,
            physical: best.physical,
            raw,
            graphics_family: best.graphics_family,
            graphics_queue,
            features: best.features,
            mesh_limits,
            timestamp_period_ns,
            limits,
            name: best.name,
            swapchain_loader,
            mesh_loader,
            acceleration_loader,
            scratch_alignment,
            debug_utils,
            allocator: Mutex::new(Some(allocator)),
            bindless: Mutex::new(Some(bindless)),
            bindless_layout,
            bindless_set,
            dlss,
            memory_counters: Default::default(),
            budget_cap: std::env::var("FORGE_VRAM_BUDGET_MB")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .map(|mb| mb << 20),
        }))
    }

    /// The PCI vendor id of the GPU [`Device::with_options`] would select on `instance` (for
    /// `surface`), without creating a device, or `None` when no GPU qualifies. The app asks a
    /// plain instance first, so NVIDIA Streamline's interposer is loaded only for an NVIDIA GPU
    /// (issue #67).
    pub fn preferred_vendor(
        instance: &Instance,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Option<u32>> {
        Ok(Self::best_candidate(instance, surface)?.map(|c| c.vendor_id))
    }

    /// The highest-scoring GPU of `instance` that meets the baseline (the last of equals).
    fn best_candidate(
        instance: &Instance,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Option<Candidate>> {
        // SAFETY: plain enumeration on a live instance.
        let physicals = unsafe { instance.raw().enumerate_physical_devices()? };
        let mut candidates: Vec<Candidate> = Vec::new();
        for physical in physicals {
            if let Some(c) = Self::evaluate(instance, physical, surface)? {
                candidates.push(c);
            }
        }
        Ok(candidates.into_iter().max_by_key(|c| c.score))
    }

    fn evaluate(
        instance: &Instance,
        physical: vk::PhysicalDevice,
        surface: Option<vk::SurfaceKHR>,
    ) -> Result<Option<Candidate>> {
        let raw = instance.raw();
        // SAFETY: property/feature queries on a valid physical device.
        let (props, extensions, families) = unsafe {
            (
                raw.get_physical_device_properties(physical),
                raw.enumerate_device_extension_properties(physical)?,
                raw.get_physical_device_queue_family_properties(physical),
            )
        };
        let name = props
            .device_name_as_c_str()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if props.api_version < vk::API_VERSION_1_3 {
            tracing::info!(device = %name, "skipped: Vulkan 1.3 required");
            return Ok(None);
        }
        let has = |ext: &CStr| {
            extensions
                .iter()
                .any(|e| e.extension_name_as_c_str().is_ok_and(|n| n == ext))
        };
        if surface.is_some() && !has(khr::swapchain::NAME) {
            return Ok(None);
        }

        let mut graphics_family = None;
        for (index, family) in families.iter().enumerate() {
            let index = index as u32;
            if !family
                .queue_flags
                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
            {
                continue;
            }
            if let Some(surface) = surface {
                // SAFETY: valid surface and family index.
                let present = unsafe {
                    instance
                        .surface_loader()
                        .get_physical_device_surface_support(physical, index, surface)?
                };
                if !present {
                    continue;
                }
            }
            graphics_family = Some(index);
            break;
        }
        let Some(graphics_family) = graphics_family else {
            return Ok(None);
        };

        let mut v11 = vk::PhysicalDeviceVulkan11Features::default();
        let mut v12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut v13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut mesh = vk::PhysicalDeviceMeshShaderFeaturesEXT::default();
        let mut features2 = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut v11)
            .push_next(&mut v12)
            .push_next(&mut v13);
        let mesh_ext = has(ext::mesh_shader::NAME);
        if mesh_ext {
            features2 = features2.push_next(&mut mesh);
        }
        let uint8_ext = [vk::KHR_INDEX_TYPE_UINT8_NAME, vk::EXT_INDEX_TYPE_UINT8_NAME]
            .into_iter()
            .find(|name| has(name));
        let mut uint8 = vk::PhysicalDeviceIndexTypeUint8FeaturesKHR::default();
        if uint8_ext.is_some() {
            features2 = features2.push_next(&mut uint8);
        }
        // SAFETY: feature query with a properly chained struct.
        unsafe { raw.get_physical_device_features2(physical, &mut features2) };
        let base = features2.features;
        let fragment_stores_and_atomics = base.fragment_stores_and_atomics;
        // Everything `with_options` enables unconditionally (issue #67): a device without one of
        // them is skipped by name here, rather than failing at device creation.
        let on = |b: vk::Bool32| b == vk::TRUE;
        let required = [
            ("shaderInt64", on(base.shader_int64)),
            ("shaderInt16", on(base.shader_int16)),
            ("samplerAnisotropy", on(base.sampler_anisotropy)),
            ("multiDrawIndirect", on(base.multi_draw_indirect)),
            (
                "drawIndirectFirstInstance",
                on(base.draw_indirect_first_instance),
            ),
            ("fillModeNonSolid", on(base.fill_mode_non_solid)),
            ("geometryShader", on(base.geometry_shader)),
            ("shaderDrawParameters", on(v11.shader_draw_parameters)),
            ("timelineSemaphore", on(v12.timeline_semaphore)),
            ("bufferDeviceAddress", on(v12.buffer_device_address)),
            ("descriptorIndexing", on(v12.descriptor_indexing)),
            ("runtimeDescriptorArray", on(v12.runtime_descriptor_array)),
            (
                "shaderSampledImageArrayNonUniformIndexing",
                on(v12.shader_sampled_image_array_non_uniform_indexing),
            ),
            (
                "shaderStorageBufferArrayNonUniformIndexing",
                on(v12.shader_storage_buffer_array_non_uniform_indexing),
            ),
            (
                "shaderStorageImageArrayNonUniformIndexing",
                on(v12.shader_storage_image_array_non_uniform_indexing),
            ),
            (
                "descriptorBindingPartiallyBound",
                on(v12.descriptor_binding_partially_bound),
            ),
            (
                "descriptorBindingSampledImageUpdateAfterBind",
                on(v12.descriptor_binding_sampled_image_update_after_bind),
            ),
            (
                "descriptorBindingStorageImageUpdateAfterBind",
                on(v12.descriptor_binding_storage_image_update_after_bind),
            ),
            (
                "descriptorBindingStorageBufferUpdateAfterBind",
                on(v12.descriptor_binding_storage_buffer_update_after_bind),
            ),
            (
                "descriptorBindingUpdateUnusedWhilePending",
                on(v12.descriptor_binding_update_unused_while_pending),
            ),
            (
                "descriptorBindingVariableDescriptorCount",
                on(v12.descriptor_binding_variable_descriptor_count),
            ),
            ("scalarBlockLayout", on(v12.scalar_block_layout)),
            ("hostQueryReset", on(v12.host_query_reset)),
            ("drawIndirectCount", on(v12.draw_indirect_count)),
            ("shaderFloat16", on(v12.shader_float16)),
            ("shaderInt8", on(v12.shader_int8)),
            (
                "storageBuffer8BitAccess",
                on(v12.storage_buffer8_bit_access),
            ),
            (
                "uniformAndStorageBuffer8BitAccess",
                on(v12.uniform_and_storage_buffer8_bit_access),
            ),
            ("dynamicRendering", on(v13.dynamic_rendering)),
            ("synchronization2", on(v13.synchronization2)),
            ("maintenance4", on(v13.maintenance4)),
            (
                "shaderDemoteToHelperInvocation",
                on(v13.shader_demote_to_helper_invocation),
            ),
            ("subgroupSizeControl", on(v13.subgroup_size_control)),
            ("privateData", on(v13.private_data)),
        ];
        let missing: Vec<&str> = required
            .iter()
            .filter(|(_, present)| !present)
            .map(|(feature, _)| *feature)
            .collect();
        if !missing.is_empty() {
            tracing::info!(device = %name, ?missing, "skipped: missing required features");
            return Ok(None);
        }
        // The bindless set's sizes (`bindless.rs`) against the update-after-bind limits.
        let mut v12_props = vk::PhysicalDeviceVulkan12Properties::default();
        let mut props2 = vk::PhysicalDeviceProperties2::default().push_next(&mut v12_props);
        // SAFETY: property query with a properly chained struct.
        unsafe { raw.get_physical_device_properties2(physical, &mut props2) };
        let (sampled, storage) = crate::bindless::image_capacity();
        if v12_props.max_descriptor_set_update_after_bind_sampled_images < sampled
            || v12_props.max_per_stage_descriptor_update_after_bind_sampled_images < sampled
            || v12_props.max_descriptor_set_update_after_bind_storage_images < storage
            || v12_props.max_per_stage_descriptor_update_after_bind_storage_images < storage
        {
            tracing::info!(
                device = %name,
                sampled,
                storage,
                "skipped: the bindless set exceeds its update-after-bind limits"
            );
            return Ok(None);
        }
        let features = DeviceFeatures {
            mesh_shader: mesh_ext && mesh.mesh_shader == vk::TRUE,
            // Slang declares SPV_KHR_ray_tracing next to SPV_KHR_ray_query for a structure reached
            // by address (OpConvertUToAccelerationStructureKHR), which the ray-tracing pipeline
            // extension covers: the three come together.
            ray_query: has(khr::ray_query::NAME)
                && has(khr::acceleration_structure::NAME)
                && has(khr::ray_tracing_pipeline::NAME),
            sampler_minmax: has(ext::sampler_filter_minmax::NAME)
                && v12.sampler_filter_minmax == vk::TRUE,
            memory_budget: has(ext::memory_budget::NAME),
            index_type_uint8: uint8_ext.is_some() && uint8.index_type_uint8 == vk::TRUE,
            int64_atomics: v12.shader_buffer_int64_atomics == vk::TRUE
                && fragment_stores_and_atomics == vk::TRUE,
        };
        let mut score = match props.device_type {
            vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 100,
            _ => 10,
        };
        if features.mesh_shader {
            score += 50;
        }
        if features.ray_query {
            score += 20;
        }
        Ok(Some(Candidate {
            physical,
            graphics_family,
            features,
            index_type_uint8: uint8_ext.filter(|_| features.index_type_uint8),
            score,
            name,
            vendor_id: props.vendor_id,
        }))
    }

    /// DLSS, when the instance was created through Streamline
    /// ([`Instance::with_streamline`]) and this GPU runs it.
    pub fn dlss(&self) -> Option<&crate::dlss::Dlss> {
        self.dlss.as_ref()
    }

    /// Alignment of an acceleration structure build's scratch address, from the device
    /// (`minAccelerationStructureScratchOffsetAlignment`; issue #67).
    pub fn scratch_alignment(&self) -> u64 {
        self.scratch_alignment
    }

    /// The raw device.
    pub fn raw(&self) -> &ash::Device {
        &self.raw
    }

    /// The instance this device was created from.
    pub fn instance(&self) -> &Arc<Instance> {
        &self.instance
    }

    /// The physical device.
    pub fn physical(&self) -> vk::PhysicalDevice {
        self.physical
    }

    /// Graphics + compute + present queue.
    pub fn graphics_queue(&self) -> vk::Queue {
        self.graphics_queue
    }

    /// Family index of [`Self::graphics_queue`].
    pub fn graphics_family(&self) -> u32 {
        self.graphics_family
    }

    /// Optional features detected.
    pub fn features(&self) -> DeviceFeatures {
        self.features
    }

    /// Mesh-shader limits when the feature is present.
    pub fn mesh_limits(&self) -> Option<MeshShaderLimits> {
        self.mesh_limits
    }

    /// The physical device's limits.
    pub fn limits(&self) -> &vk::PhysicalDeviceLimits {
        &self.limits
    }

    /// Nanoseconds per timestamp tick.
    pub fn timestamp_period_ns(&self) -> f32 {
        self.timestamp_period_ns
    }

    /// Device name reported by the driver.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Swapchain functions.
    pub fn swapchain_loader(&self) -> &khr::swapchain::Device {
        &self.swapchain_loader
    }

    /// Mesh-shader functions when available.
    pub fn mesh_loader(&self) -> Option<&ext::mesh_shader::Device> {
        self.mesh_loader.as_ref()
    }

    /// The acceleration-structure functions, on a device with ray queries.
    pub fn acceleration_loader(&self) -> Option<&khr::acceleration_structure::Device> {
        self.acceleration_loader.as_ref()
    }

    pub(crate) fn memory_counters(&self) -> &crate::memory_report::MemoryCounters {
        &self.memory_counters
    }

    pub(crate) fn budget_cap(&self) -> Option<u64> {
        self.budget_cap
    }

    pub(crate) fn with_allocator<R>(&self, f: impl FnOnce(&mut Allocator) -> R) -> R {
        let mut guard = self.allocator.lock();
        f(guard.as_mut().expect("allocator alive"))
    }

    fn with_bindless<R>(&self, f: impl FnOnce(&Bindless) -> R) -> R {
        let guard = self.bindless.lock();
        f(guard.as_ref().expect("bindless set alive"))
    }

    /// Layout of the global bindless set (set 0 of every pipeline).
    pub fn bindless_layout(&self) -> vk::DescriptorSetLayout {
        self.bindless_layout
    }

    /// The global bindless set.
    pub fn bindless_set(&self) -> vk::DescriptorSet {
        self.bindless_set
    }

    /// Makes an image view visible to shaders as a sampled image; returns its index.
    pub fn register_sampled_image(
        &self,
        view: vk::ImageView,
        layout: vk::ImageLayout,
    ) -> SampledImageId {
        self.with_bindless(|b| b.register_sampled(&self.raw, view, layout))
    }

    /// Makes an image view visible to shaders as a storage image (layout `GENERAL`).
    pub fn register_storage_image(&self, view: vk::ImageView) -> StorageImageId {
        self.with_bindless(|b| b.register_storage(&self.raw, view))
    }

    /// Frees a sampled-image slot. No in-flight frame may still read it.
    pub fn release_sampled_image(&self, id: SampledImageId) {
        self.with_bindless(|b| b.release_sampled(id));
    }

    /// Frees a storage-image slot. No in-flight frame may still read it.
    pub fn release_storage_image(&self, id: StorageImageId) {
        self.with_bindless(|b| b.release_storage(id));
    }

    /// Names an object for validation messages and RenderDoc (no-op without validation).
    pub fn set_name<H: vk::Handle>(&self, handle: H, name: &str) {
        if let Some(debug) = &self.debug_utils
            && let Ok(name) = std::ffi::CString::new(name)
        {
            let info = vk::DebugUtilsObjectNameInfoEXT::default()
                .object_handle(handle)
                .object_name(&name);
            // SAFETY: the handle belongs to this device.
            let _ = unsafe { debug.set_debug_utils_object_name(&info) };
        }
    }

    /// Blocks until the GPU is idle. Only for shutdown and resource teardown.
    pub fn wait_idle(&self) {
        // SAFETY: plain wait on a live device.
        let _ = unsafe { self.raw.device_wait_idle() };
    }

    /// Records `f` into a one-shot command buffer, submits it and waits. Initialisation only:
    /// never call this inside a frame.
    pub fn execute_transient(&self, f: impl FnOnce(&ash::Device, vk::CommandBuffer)) -> Result<()> {
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.graphics_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        // SAFETY: standard one-shot command buffer lifecycle; everything created here is
        // destroyed before returning and the fence wait orders the destruction after use.
        unsafe {
            let pool = self.raw.create_command_pool(&pool_info, None)?;
            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let cb = self.raw.allocate_command_buffers(&alloc)?[0];
            self.raw.begin_command_buffer(
                cb,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            f(&self.raw, cb);
            self.raw.end_command_buffer(cb)?;
            let fence = self
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)?;
            let cbs = [vk::CommandBufferSubmitInfo::default().command_buffer(cb)];
            let submit = vk::SubmitInfo2::default().command_buffer_infos(&cbs);
            self.raw
                .queue_submit2(self.graphics_queue, &[submit], fence)?;
            self.raw.wait_for_fences(&[fence], true, u64::MAX)?;
            self.raw.destroy_fence(fence, None);
            self.raw.destroy_command_pool(pool, None);
        }
        Ok(())
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.wait_idle();
        // Streamline frees what it allocated on the device first.
        #[cfg(all(feature = "dlss", windows))]
        if let Some(streamline) = self.instance.streamline() {
            streamline.shut_down();
        }
        if let Some(mut bindless) = self.bindless.lock().take() {
            bindless.destroy(&self.raw);
        }
        drop(self.allocator.lock().take());
        // SAFETY: all resources were released (the allocator was just dropped) and the GPU is idle.
        unsafe { self.raw.destroy_device(None) };
    }
}

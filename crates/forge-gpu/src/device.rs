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
    /// `VK_KHR_ray_query` + acceleration structures.
    pub ray_query: bool,
    /// `VK_EXT_sampler_filter_minmax` (min-reduction sampling for depth pyramids).
    pub sampler_minmax: bool,
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
    name: String,
    swapchain_loader: khr::swapchain::Device,
    mesh_loader: Option<ext::mesh_shader::Device>,
    debug_utils: Option<ext::debug_utils::Device>,
    allocator: Mutex<Option<Allocator>>,
    bindless: Mutex<Option<Bindless>>,
    bindless_layout: vk::DescriptorSetLayout,
    bindless_set: vk::DescriptorSet,
    /// DLSS, when the instance came through Streamline and this GPU runs it.
    dlss: Option<crate::dlss::Dlss>,
}

struct Candidate {
    physical: vk::PhysicalDevice,
    graphics_family: u32,
    features: DeviceFeatures,
    score: u32,
    name: String,
}

impl Device {
    /// Picks the best physical device that can present to `surface` (when given) and creates
    /// the logical device with the engine's baseline features plus the optional ones found.
    pub fn new(instance: Arc<Instance>, surface: Option<vk::SurfaceKHR>) -> Result<Arc<Self>> {
        let raw_instance = instance.raw();
        // SAFETY: plain enumeration on a live instance.
        let physicals = unsafe { raw_instance.enumerate_physical_devices()? };
        let mut candidates: Vec<Candidate> = Vec::new();
        for physical in physicals {
            if let Some(c) = Self::evaluate(&instance, physical, surface)? {
                candidates.push(c);
            }
        }
        let best = candidates
            .into_iter()
            .max_by_key(|c| c.score)
            .ok_or_else(|| {
                GpuError::Unsupported("no Vulkan 1.3 device with a graphics queue".into())
            })?;
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

        let base = vk::PhysicalDeviceFeatures::default()
            .shader_int64(true)
            .shader_int16(true)
            .sampler_anisotropy(true)
            .multi_draw_indirect(true)
            .fill_mode_non_solid(true)
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
        let mut mesh = vk::PhysicalDeviceMeshShaderFeaturesEXT::default()
            .task_shader(true)
            .mesh_shader(true);
        let mut features2 = vk::PhysicalDeviceFeatures2::default()
            .features(base)
            .push_next(&mut v11)
            .push_next(&mut v12)
            .push_next(&mut v13);
        if best.features.mesh_shader {
            features2 = features2.push_next(&mut mesh);
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
        let mut props2 = vk::PhysicalDeviceProperties2::default();
        if best.features.mesh_shader {
            props2 = props2.push_next(&mut mesh_props);
        }
        // SAFETY: property query on a valid physical device.
        unsafe { raw_instance.get_physical_device_properties2(best.physical, &mut props2) };
        let timestamp_period_ns = props2.properties.limits.timestamp_period;
        let max_anisotropy = props2.properties.limits.max_sampler_anisotropy;
        let mesh_limits = best.features.mesh_shader.then_some(MeshShaderLimits {
            preferred_task_invocations: mesh_props.max_preferred_task_work_group_invocations,
            preferred_mesh_invocations: mesh_props.max_preferred_mesh_work_group_invocations,
            max_output_vertices: mesh_props.max_mesh_output_vertices,
            max_output_primitives: mesh_props.max_mesh_output_primitives,
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
            name: best.name,
            swapchain_loader,
            mesh_loader,
            debug_utils,
            allocator: Mutex::new(Some(allocator)),
            bindless: Mutex::new(Some(bindless)),
            bindless_layout,
            bindless_set,
            dlss,
        }))
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

        let mut v12 = vk::PhysicalDeviceVulkan12Features::default();
        let mut v13 = vk::PhysicalDeviceVulkan13Features::default();
        let mut mesh = vk::PhysicalDeviceMeshShaderFeaturesEXT::default();
        let mut features2 = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut v12)
            .push_next(&mut v13);
        let mesh_ext = has(ext::mesh_shader::NAME);
        if mesh_ext {
            features2 = features2.push_next(&mut mesh);
        }
        // SAFETY: feature query with a properly chained struct.
        unsafe { raw.get_physical_device_features2(physical, &mut features2) };
        let baseline = v12.timeline_semaphore == vk::TRUE
            && v12.buffer_device_address == vk::TRUE
            && v12.descriptor_indexing == vk::TRUE
            && v13.dynamic_rendering == vk::TRUE
            && v13.synchronization2 == vk::TRUE;
        if !baseline {
            tracing::info!(device = %name, "skipped: missing baseline features");
            return Ok(None);
        }
        let features = DeviceFeatures {
            mesh_shader: mesh_ext && mesh.task_shader == vk::TRUE && mesh.mesh_shader == vk::TRUE,
            ray_query: has(khr::ray_query::NAME) && has(khr::acceleration_structure::NAME),
            sampler_minmax: has(ext::sampler_filter_minmax::NAME)
                && v12.sampler_filter_minmax == vk::TRUE,
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
            score,
            name,
        }))
    }

    /// DLSS, when the instance was created through Streamline
    /// ([`Instance::with_streamline`]) and this GPU runs it.
    pub fn dlss(&self) -> Option<&crate::dlss::Dlss> {
        self.dlss.as_ref()
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

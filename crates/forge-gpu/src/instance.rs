use std::ffi::{CStr, c_void};
use std::sync::Arc;

use ash::{ext, khr, vk};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::error::Result;

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// A Vulkan instance with optional validation and debug messenger.
pub struct Instance {
    entry: ash::Entry,
    raw: ash::Instance,
    surface_loader: khr::surface::Instance,
    debug: Option<(ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    validation: bool,
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    // SAFETY: Vulkan guarantees `data` points to a valid callback-data struct whose strings are
    // NUL-terminated for the duration of the call.
    let (message, id) = unsafe {
        let data = &*data;
        (
            data.message_as_c_str()
                .map(|m| m.to_string_lossy().into_owned())
                .unwrap_or_default(),
            data.message_id_name_as_c_str()
                .map(|m| m.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    };
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        tracing::error!(target: "vulkan", %id, ?kind, "{message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        tracing::warn!(target: "vulkan", %id, ?kind, "{message}");
    } else {
        tracing::debug!(target: "vulkan", %id, ?kind, "{message}");
    }
    vk::FALSE
}

impl Instance {
    /// Creates the instance. `validation` turns on the Khronos validation layer when it is
    /// installed; `display` adds the surface extensions the window system needs.
    pub fn new(
        app_name: &CStr,
        validation: bool,
        display: Option<RawDisplayHandle>,
    ) -> Result<Self> {
        // SAFETY: loading the Vulkan library has no preconditions beyond it being installed.
        let entry = unsafe { ash::Entry::load()? };
        // SAFETY: plain property enumeration.
        let layers = unsafe { entry.enumerate_instance_layer_properties()? };
        let has_validation = layers
            .iter()
            .any(|l| l.layer_name_as_c_str().is_ok_and(|n| n == VALIDATION_LAYER));
        let validation = validation && has_validation;
        if validation {
            tracing::info!("Vulkan validation layer enabled");
        }

        let mut extensions: Vec<*const i8> = Vec::new();
        if let Some(display) = display {
            extensions.extend_from_slice(ash_window::enumerate_required_extensions(display)?);
        }
        if validation {
            extensions.push(ext::debug_utils::NAME.as_ptr());
        }
        let layer_names = if validation {
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };

        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name)
            .application_version(1)
            .engine_name(c"forge")
            .engine_version(1)
            .api_version(vk::API_VERSION_1_3);
        let mut messenger_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::ERROR
                    | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE
                    | vk::DebugUtilsMessageTypeFlagsEXT::GENERAL,
            )
            .pfn_user_callback(Some(debug_callback));
        // `FORGE_SYNC_VALIDATION=1` adds the layer's synchronization validation (hazard
        // detection across barriers); it is slow, so it is opt-in even under `--validate`.
        let sync_validation =
            validation && std::env::var_os("FORGE_SYNC_VALIDATION").is_some_and(|v| v != "0");
        // `FORGE_GPU_AV=1` adds GPU-assisted validation (out-of-bounds buffer-device-address
        // and descriptor accesses are caught on the GPU); also slow, also opt-in.
        let gpu_av = validation && std::env::var_os("FORGE_GPU_AV").is_some_and(|v| v != "0");
        let mut enabled_features = Vec::new();
        if sync_validation {
            enabled_features.push(vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION);
            tracing::info!("Vulkan synchronization validation enabled");
        }
        if gpu_av {
            enabled_features.push(vk::ValidationFeatureEnableEXT::GPU_ASSISTED);
            enabled_features
                .push(vk::ValidationFeatureEnableEXT::GPU_ASSISTED_RESERVE_BINDING_SLOT);
            tracing::info!("Vulkan GPU-assisted validation enabled");
        }
        let mut validation_features =
            vk::ValidationFeaturesEXT::default().enabled_validation_features(&enabled_features);
        let mut info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(&extensions)
            .enabled_layer_names(&layer_names);
        if validation {
            info = info.push_next(&mut messenger_info);
        }
        if !enabled_features.is_empty() {
            info = info.push_next(&mut validation_features);
        }
        // SAFETY: every pointer in `info` refers to data that outlives the call.
        let raw = unsafe { entry.create_instance(&info, None)? };
        let surface_loader = khr::surface::Instance::new(&entry, &raw);
        let debug = if validation {
            let loader = ext::debug_utils::Instance::new(&entry, &raw);
            // SAFETY: the messenger info is valid and the callback is `extern "system"`.
            let messenger = unsafe { loader.create_debug_utils_messenger(&messenger_info, None)? };
            Some((loader, messenger))
        } else {
            None
        };
        Ok(Self {
            entry,
            raw,
            surface_loader,
            debug,
            validation,
        })
    }

    /// The loaded entry points.
    pub fn entry(&self) -> &ash::Entry {
        &self.entry
    }

    /// The raw instance.
    pub fn raw(&self) -> &ash::Instance {
        &self.raw
    }

    /// Surface extension functions.
    pub fn surface_loader(&self) -> &khr::surface::Instance {
        &self.surface_loader
    }

    /// Whether validation is active (debug names are only set when it is).
    pub fn validation_enabled(&self) -> bool {
        self.validation
    }

    /// Creates a window surface. The window must outlive it.
    pub fn create_surface(
        self: &Arc<Self>,
        display: RawDisplayHandle,
        window: RawWindowHandle,
    ) -> Result<Arc<Surface>> {
        // SAFETY: the handles come from a live window that the caller keeps alive.
        let raw =
            unsafe { ash_window::create_surface(&self.entry, &self.raw, display, window, None)? };
        Ok(Arc::new(Surface {
            instance: Arc::clone(self),
            raw,
        }))
    }
}

/// A window surface, destroyed on drop after every swapchain that used it (they hold an `Arc`).
pub struct Surface {
    instance: Arc<Instance>,
    raw: vk::SurfaceKHR,
}

impl Surface {
    /// The Vulkan handle.
    pub fn raw(&self) -> vk::SurfaceKHR {
        self.raw
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: swapchains keep the surface alive, so none uses it any more.
        unsafe { self.instance.surface_loader.destroy_surface(self.raw, None) };
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: every device created from this instance has been dropped first (they hold
        // an `Arc<Instance>`).
        unsafe {
            if let Some((loader, messenger)) = self.debug.take() {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}

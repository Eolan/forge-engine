use std::sync::Arc;

use ash::vk;

use crate::device::Device;
use crate::error::Result;
use crate::instance::Surface;

/// The window's presentable images.
pub struct Swapchain {
    device: Arc<Device>,
    surface: Arc<Surface>,
    raw: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    vsync: bool,
}

impl Swapchain {
    /// Creates a swapchain for `surface`. `vsync` selects FIFO; otherwise mailbox/immediate.
    pub fn new(
        device: Arc<Device>,
        surface: Arc<Surface>,
        width: u32,
        height: u32,
        vsync: bool,
    ) -> Result<Self> {
        let mut this = Self {
            device,
            surface,
            raw: vk::SwapchainKHR::null(),
            images: Vec::new(),
            views: Vec::new(),
            format: vk::Format::UNDEFINED,
            extent: vk::Extent2D::default(),
            vsync,
        };
        this.recreate(width, height)?;
        Ok(this)
    }

    /// Rebuilds the swapchain for a new size (waits for the device to be idle).
    pub fn recreate(&mut self, width: u32, height: u32) -> Result<()> {
        self.device.wait_idle();
        let instance = self.device.instance();
        let loader = instance.surface_loader();
        let physical = self.device.physical();
        let surface = self.surface.raw();
        // SAFETY: surface queries on a live surface and physical device.
        let (caps, formats, modes) = unsafe {
            (
                loader.get_physical_device_surface_capabilities(physical, surface)?,
                loader.get_physical_device_surface_formats(physical, surface)?,
                loader.get_physical_device_surface_present_modes(physical, surface)?,
            )
        };
        let format = formats
            .iter()
            .find(|f| {
                (f.format == vk::Format::B8G8R8A8_SRGB || f.format == vk::Format::R8G8B8A8_SRGB)
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .or_else(|| formats.first())
            .copied()
            .ok_or_else(|| crate::GpuError::Unsupported("surface has no formats".into()))?;
        // `FORGE_PRESENT_MODE=immediate|mailbox|fifo` overrides the choice (debugging aid).
        let forced = match std::env::var("FORGE_PRESENT_MODE").ok().as_deref() {
            Some("immediate") => Some(vk::PresentModeKHR::IMMEDIATE),
            Some("mailbox") => Some(vk::PresentModeKHR::MAILBOX),
            Some("fifo") => Some(vk::PresentModeKHR::FIFO),
            _ => None,
        }
        .filter(|mode| modes.contains(mode));
        let present_mode = if let Some(mode) = forced {
            mode
        } else if self.vsync {
            vk::PresentModeKHR::FIFO
        } else if instance.through_streamline() && modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
            // Through Streamline's interposer, MAILBOX presents were held to the display's
            // refresh in most runs (the acquire waited 8 ms at 120 Hz); IMMEDIATE never was.
            vk::PresentModeKHR::IMMEDIATE
        } else if modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
            vk::PresentModeKHR::IMMEDIATE
        } else {
            vk::PresentModeKHR::FIFO
        };
        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };
        let mut image_count = caps.min_image_count + 1;
        if caps.max_image_count > 0 {
            image_count = image_count.min(caps.max_image_count);
        }
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(self.raw);
        // SAFETY: valid create info; the old swapchain is retired below after the device is idle.
        let raw = unsafe {
            self.device
                .swapchain_loader()
                .create_swapchain(&info, None)?
        };
        self.destroy_views_and_swapchain();
        self.raw = raw;
        // SAFETY: `raw` is the live swapchain just created.
        self.images = unsafe { self.device.swapchain_loader().get_swapchain_images(raw)? };
        self.views = self
            .images
            .iter()
            .map(|&image| {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format.format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    });
                // SAFETY: valid view of a swapchain image.
                unsafe { self.device.raw().create_image_view(&view_info, None) }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        self.format = format.format;
        self.extent = extent;
        tracing::info!(?extent, ?present_mode, format = ?format.format, images = self.images.len(), "swapchain created");
        Ok(())
    }

    fn destroy_views_and_swapchain(&mut self) {
        // SAFETY: the device is idle (callers wait) so no frame uses these views or images.
        unsafe {
            for view in self.views.drain(..) {
                self.device.raw().destroy_image_view(view, None);
            }
            if self.raw != vk::SwapchainKHR::null() {
                self.device
                    .swapchain_loader()
                    .destroy_swapchain(self.raw, None);
                self.raw = vk::SwapchainKHR::null();
            }
        }
        self.images.clear();
    }

    /// Acquires the next image, signalling `signal` when it is ready. Returns `None` when the
    /// swapchain is out of date and must be recreated.
    pub fn acquire(&self, signal: vk::Semaphore) -> Result<Option<u32>> {
        // SAFETY: live swapchain and semaphore.
        match unsafe {
            self.device.swapchain_loader().acquire_next_image(
                self.raw,
                u64::MAX,
                signal,
                vk::Fence::null(),
            )
        } {
            Ok((index, _suboptimal)) => Ok(Some(index)),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Presents `index` after `wait` is signalled. Returns `true` when the swapchain should be
    /// recreated (suboptimal or out of date).
    pub fn present(&self, wait: vk::Semaphore, index: u32) -> Result<bool> {
        let waits = [wait];
        let swapchains = [self.raw];
        let indices = [index];
        let info = vk::PresentInfoKHR::default()
            .wait_semaphores(&waits)
            .swapchains(&swapchains)
            .image_indices(&indices);
        // SAFETY: the image was acquired and rendering into it is ordered by `wait`.
        match unsafe {
            self.device
                .swapchain_loader()
                .queue_present(self.device.graphics_queue(), &info)
        } {
            Ok(suboptimal) => Ok(suboptimal),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Ok(true),
            Err(e) => Err(e.into()),
        }
    }

    /// Image `index`.
    pub fn image(&self, index: u32) -> vk::Image {
        self.images[index as usize]
    }
    /// View of image `index`.
    pub fn view(&self, index: u32) -> vk::ImageView {
        self.views[index as usize]
    }
    /// Number of images.
    pub fn image_count(&self) -> usize {
        self.images.len()
    }
    /// Pixel format.
    pub fn format(&self) -> vk::Format {
        self.format
    }
    /// Size in pixels.
    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        self.device.wait_idle();
        self.destroy_views_and_swapchain();
    }
}

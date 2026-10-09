use std::sync::Arc;

use ash::vk;

use crate::bindless::SampledImageId;
use crate::device::Device;
use crate::error::Result;
use crate::instance::Surface;

/// What a swapchain presents (issue #94).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SurfaceMode {
    /// 8-bit sRGB.
    #[default]
    Sdr,
    /// HDR10: Rec.2100 PQ over Rec.2020 primaries in 10 bits (`HDR10_ST2084_EXT`).
    Hdr10,
    /// scRGB: linear Rec.709 in half floats, 1.0 being 80 nits (`EXTENDED_SRGB_LINEAR_EXT`).
    ScRgb,
}

impl SurfaceMode {
    /// Whether `format` is one this mode presents.
    fn accepts(self, format: &vk::SurfaceFormatKHR) -> bool {
        match self {
            SurfaceMode::Sdr => {
                (format.format == vk::Format::B8G8R8A8_SRGB
                    || format.format == vk::Format::R8G8B8A8_SRGB)
                    && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            }
            SurfaceMode::Hdr10 => {
                (format.format == vk::Format::A2B10G10R10_UNORM_PACK32
                    || format.format == vk::Format::A2R10G10B10_UNORM_PACK32)
                    && format.color_space == vk::ColorSpaceKHR::HDR10_ST2084_EXT
            }
            SurfaceMode::ScRgb => {
                format.format == vk::Format::R16G16B16A16_SFLOAT
                    && format.color_space == vk::ColorSpaceKHR::EXTENDED_SRGB_LINEAR_EXT
            }
        }
    }

    /// Short name for logs and overlays.
    pub fn name(self) -> &'static str {
        match self {
            SurfaceMode::Sdr => "SDR",
            SurfaceMode::Hdr10 => "HDR10",
            SurfaceMode::ScRgb => "scRGB",
        }
    }
}

/// What an HDR swapchain tells the display of its content (`VK_EXT_hdr_metadata`, CTA-861.3),
/// in nits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrMetadata {
    /// The mastering display's peak: the tone curve's.
    pub peak: f32,
    /// The mastering display's black.
    pub black: f32,
    /// MaxCLL: the brightest pixel's largest channel.
    pub max_cll: f32,
    /// MaxFALL: the largest frame average of the pixels' largest channels; 0 when unknown.
    pub max_fall: f32,
}

/// The window's presentable images.
pub struct Swapchain {
    device: Arc<Device>,
    surface: Arc<Surface>,
    raw: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    /// The images' sampled handles, in the HDR modes when the surface allows it (issue #125:
    /// the metadata histogram reads the frame there).
    sampled: Vec<SampledImageId>,
    format: vk::Format,
    extent: vk::Extent2D,
    vsync: bool,
    /// The mode asked for, and the one the surface gave.
    requested: SurfaceMode,
    mode: SurfaceMode,
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
            sampled: Vec::new(),
            format: vk::Format::UNDEFINED,
            extent: vk::Extent2D::default(),
            vsync,
            requested: SurfaceMode::Sdr,
            mode: SurfaceMode::Sdr,
        };
        this.recreate(width, height)?;
        Ok(this)
    }

    /// The modes the surface offers (SDR always).
    pub fn supported_modes(&self) -> Result<Vec<SurfaceMode>> {
        // SAFETY: a surface query on a live surface and physical device.
        let formats = unsafe {
            self.device
                .instance()
                .surface_loader()
                .get_physical_device_surface_formats(self.device.physical(), self.surface.raw())?
        };
        Ok([SurfaceMode::Sdr, SurfaceMode::Hdr10, SurfaceMode::ScRgb]
            .into_iter()
            .filter(|&mode| mode == SurfaceMode::Sdr || formats.iter().any(|f| mode.accepts(f)))
            .collect())
    }

    /// Asks for `mode` and recreates the swapchain (waits for the device to be idle). Returns
    /// the mode the surface gave: SDR when it does not offer the one asked for.
    pub fn set_mode(&mut self, mode: SurfaceMode) -> Result<SurfaceMode> {
        self.requested = mode;
        let extent = self.extent;
        self.recreate(extent.width, extent.height)?;
        Ok(self.mode)
    }

    /// The mode presented.
    pub fn mode(&self) -> SurfaceMode {
        self.mode
    }

    /// Describes the HDR content to the display (`VK_EXT_hdr_metadata`, when the device has
    /// it; issues #94, #125): Rec.2020 primaries, a D65 white and `metadata`. SDR swapchains
    /// are left alone.
    pub fn set_hdr_metadata(&self, metadata: HdrMetadata) {
        let Some(loader) = self.device.hdr_metadata_loader() else {
            return;
        };
        if self.mode == SurfaceMode::Sdr {
            return;
        }
        let xy = |x, y| vk::XYColorEXT { x, y };
        let raw = vk::HdrMetadataEXT::default()
            .display_primary_red(xy(0.708, 0.292))
            .display_primary_green(xy(0.170, 0.797))
            .display_primary_blue(xy(0.131, 0.046))
            .white_point(xy(0.3127, 0.3290))
            .max_luminance(metadata.peak)
            .min_luminance(metadata.black)
            .max_content_light_level(metadata.max_cll)
            .max_frame_average_light_level(metadata.max_fall);
        // SAFETY: a live swapchain created on this device.
        unsafe { loader.set_hdr_metadata(&[self.raw], &[raw]) };
        tracing::info!(?metadata, "HDR metadata set");
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
        let hdr = formats.iter().find(|f| self.requested.accepts(f));
        if self.requested != SurfaceMode::Sdr && hdr.is_none() {
            tracing::warn!(
                requested = self.requested.name(),
                "the surface offers no such format: SDR"
            );
        }
        let format = hdr
            .or_else(|| formats.iter().find(|f| SurfaceMode::Sdr.accepts(f)))
            .or_else(|| formats.first())
            .copied()
            .ok_or_else(|| crate::GpuError::Unsupported("surface has no formats".into()))?;
        let mode = if hdr.is_some() {
            self.requested
        } else {
            SurfaceMode::Sdr
        };
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
        // HDR images are also sampled (the metadata histogram, issue #125) when the surface and
        // the format allow it; SDR ones stay as they were.
        // SAFETY: a format query on a live physical device.
        let features = unsafe {
            instance
                .raw()
                .get_physical_device_format_properties(physical, format.format)
        }
        .optimal_tiling_features;
        let sampled = mode != SurfaceMode::Sdr
            && caps
                .supported_usage_flags
                .contains(vk::ImageUsageFlags::SAMPLED)
            && features.contains(vk::FormatFeatureFlags::SAMPLED_IMAGE);
        let mut usage = vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::TRANSFER_DST
            | vk::ImageUsageFlags::TRANSFER_SRC;
        if sampled {
            usage |= vk::ImageUsageFlags::SAMPLED;
        }
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(usage)
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
        if sampled {
            self.sampled = self
                .views
                .iter()
                .map(|&view| {
                    self.device
                        .register_sampled_image(view, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                })
                .collect();
        }
        self.format = format.format;
        self.extent = extent;
        self.mode = mode;
        tracing::info!(?extent, ?present_mode, format = ?format.format, mode = mode.name(), images = self.images.len(), sampled, "swapchain created");
        Ok(())
    }

    fn destroy_views_and_swapchain(&mut self) {
        for id in self.sampled.drain(..) {
            self.device.release_sampled_image(id);
        }
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
        let _queues = self.device.hold_queues();
        // SAFETY: the image was acquired and rendering into it is ordered by `wait`; the
        // queues are held.
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
    /// Sampled handle of image `index`, when the images are sampled (HDR modes, issue #125).
    pub fn sampled(&self, index: u32) -> Option<SampledImageId> {
        self.sampled.get(index as usize).copied()
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

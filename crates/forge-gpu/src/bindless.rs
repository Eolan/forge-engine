//! The one global descriptor set.
//!
//! Buffers are reached by device address and need no descriptors. Images do: this set holds
//! every sampled image, storage image and sampler the engine ever binds, indexed by small
//! integer handles that shaders receive inside their data. It is bound once per command
//! buffer as set 0 (`shaders/bindless.slang`); pipeline layouts include it automatically.

use std::sync::atomic::{AtomicU32, Ordering};

use ash::vk;
use parking_lot::Mutex;

use crate::error::Result;

/// Binding index of the sampled-image array.
pub const BINDING_SAMPLED_IMAGES: u32 = 0;
/// Binding index of the storage-image array.
pub const BINDING_STORAGE_IMAGES: u32 = 1;
/// Binding index of the sampler array.
pub const BINDING_SAMPLERS: u32 = 2;

const MAX_SAMPLED_IMAGES: u32 = 65_536;
const MAX_STORAGE_IMAGES: u32 = 16_384;

/// Fixed sampler slots, mirrored in `bindless.slang`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SamplerKind {
    /// Bilinear, clamp to edge.
    LinearClamp = 0,
    /// Nearest, clamp to edge.
    NearestClamp = 1,
    /// Trilinear, repeat.
    LinearRepeat = 2,
    /// Anisotropic 16×, repeat.
    AnisotropicRepeat = 3,
    /// Linear 2×2 footprint, clamp, `min` reduction (for hierarchical-Z pyramids;
    /// `VK_EXT_sampler_filter_minmax`).
    MinReductionClamp = 4,
}

const SAMPLER_COUNT: u32 = 5;

/// Handle of a registered sampled image (an index into binding 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampledImageId(pub u32);

/// Handle of a registered storage image (an index into binding 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StorageImageId(pub u32);

pub(crate) struct Bindless {
    layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    samplers: Vec<vk::Sampler>,
    next_sampled: AtomicU32,
    free_sampled: Mutex<Vec<u32>>,
    next_storage: AtomicU32,
    free_storage: Mutex<Vec<u32>>,
}

impl Bindless {
    pub(crate) fn new(raw: &ash::Device, anisotropy: f32, has_minmax: bool) -> Result<Self> {
        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(BINDING_SAMPLED_IMAGES)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(MAX_SAMPLED_IMAGES)
                .stage_flags(vk::ShaderStageFlags::ALL),
            vk::DescriptorSetLayoutBinding::default()
                .binding(BINDING_STORAGE_IMAGES)
                .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
                .descriptor_count(MAX_STORAGE_IMAGES)
                .stage_flags(vk::ShaderStageFlags::ALL),
            vk::DescriptorSetLayoutBinding::default()
                .binding(BINDING_SAMPLERS)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(SAMPLER_COUNT)
                .stage_flags(vk::ShaderStageFlags::ALL),
        ];
        let flags = [
            vk::DescriptorBindingFlags::UPDATE_AFTER_BIND
                | vk::DescriptorBindingFlags::PARTIALLY_BOUND,
            vk::DescriptorBindingFlags::UPDATE_AFTER_BIND
                | vk::DescriptorBindingFlags::PARTIALLY_BOUND,
            vk::DescriptorBindingFlags::empty(),
        ];
        let mut flags_info =
            vk::DescriptorSetLayoutBindingFlagsCreateInfo::default().binding_flags(&flags);
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default()
            .bindings(&bindings)
            .flags(vk::DescriptorSetLayoutCreateFlags::UPDATE_AFTER_BIND_POOL)
            .push_next(&mut flags_info);
        // SAFETY: valid create infos on a live device; the sizes are within the limits the
        // device advertises (checked at device selection: 1M update-after-bind images here).
        let layout = unsafe { raw.create_descriptor_set_layout(&layout_info, None)? };
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: MAX_SAMPLED_IMAGES,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_IMAGE,
                descriptor_count: MAX_STORAGE_IMAGES,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLER,
                descriptor_count: SAMPLER_COUNT,
            },
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .flags(vk::DescriptorPoolCreateFlags::UPDATE_AFTER_BIND)
            .max_sets(1)
            .pool_sizes(&sizes);
        // SAFETY: as above.
        let pool = unsafe { raw.create_descriptor_pool(&pool_info, None)? };
        let layouts = [layout];
        let alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        // SAFETY: the pool has room for exactly this set.
        let set = unsafe { raw.allocate_descriptor_sets(&alloc)?[0] };

        let mut samplers = Vec::with_capacity(SAMPLER_COUNT as usize);
        for kind in 0..SAMPLER_COUNT {
            let mut reduction = vk::SamplerReductionModeCreateInfo::default()
                .reduction_mode(vk::SamplerReductionMode::MIN);
            let mut info = vk::SamplerCreateInfo::default()
                .max_lod(vk::LOD_CLAMP_NONE)
                .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .mipmap_mode(vk::SamplerMipmapMode::LINEAR);
            match kind {
                1 => {
                    info = info
                        .mag_filter(vk::Filter::NEAREST)
                        .min_filter(vk::Filter::NEAREST)
                        .mipmap_mode(vk::SamplerMipmapMode::NEAREST);
                }
                2 => {
                    info = info
                        .address_mode_u(vk::SamplerAddressMode::REPEAT)
                        .address_mode_v(vk::SamplerAddressMode::REPEAT)
                        .address_mode_w(vk::SamplerAddressMode::REPEAT);
                }
                3 => {
                    info = info
                        .address_mode_u(vk::SamplerAddressMode::REPEAT)
                        .address_mode_v(vk::SamplerAddressMode::REPEAT)
                        .address_mode_w(vk::SamplerAddressMode::REPEAT)
                        .anisotropy_enable(anisotropy > 1.0)
                        .max_anisotropy(anisotropy.max(1.0));
                }
                4 => {
                    // Linear filtering is what gives the reduction its 2×2 footprint: with
                    // NEAREST the "minimum" is one arbitrary texel and the depth pyramid is a
                    // point-sampled downscale (that bug drew holes all over the asteroids).
                    // The level is always chosen explicitly (`SampleLevel`), so mips are NEAREST.
                    info = info
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .mipmap_mode(vk::SamplerMipmapMode::NEAREST);
                    if has_minmax {
                        info = info.push_next(&mut reduction);
                    }
                }
                _ => {}
            }
            // SAFETY: valid sampler info.
            samplers.push(unsafe { raw.create_sampler(&info, None)? });
        }
        let sampler_infos: Vec<vk::DescriptorImageInfo> = samplers
            .iter()
            .map(|&s| vk::DescriptorImageInfo::default().sampler(s))
            .collect();
        let write = vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(BINDING_SAMPLERS)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .image_info(&sampler_infos);
        // SAFETY: the set is allocated and the samplers are live.
        unsafe { raw.update_descriptor_sets(&[write], &[]) };
        Ok(Self {
            layout,
            pool,
            set,
            samplers,
            next_sampled: AtomicU32::new(0),
            free_sampled: Mutex::new(Vec::new()),
            next_storage: AtomicU32::new(0),
            free_storage: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn layout(&self) -> vk::DescriptorSetLayout {
        self.layout
    }

    pub(crate) fn set(&self) -> vk::DescriptorSet {
        self.set
    }

    pub(crate) fn register_sampled(
        &self,
        raw: &ash::Device,
        view: vk::ImageView,
        layout: vk::ImageLayout,
    ) -> SampledImageId {
        let index = self
            .free_sampled
            .lock()
            .pop()
            .unwrap_or_else(|| self.next_sampled.fetch_add(1, Ordering::Relaxed));
        assert!(
            index < MAX_SAMPLED_IMAGES,
            "bindless sampled image table full"
        );
        let info = [vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(layout)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(BINDING_SAMPLED_IMAGES)
            .dst_array_element(index)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&info);
        // SAFETY: update-after-bind allows writing unused slots while the set is bound.
        unsafe { raw.update_descriptor_sets(&[write], &[]) };
        SampledImageId(index)
    }

    pub(crate) fn register_storage(
        &self,
        raw: &ash::Device,
        view: vk::ImageView,
    ) -> StorageImageId {
        let index = self
            .free_storage
            .lock()
            .pop()
            .unwrap_or_else(|| self.next_storage.fetch_add(1, Ordering::Relaxed));
        assert!(
            index < MAX_STORAGE_IMAGES,
            "bindless storage image table full"
        );
        let info = [vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::GENERAL)];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(BINDING_STORAGE_IMAGES)
            .dst_array_element(index)
            .descriptor_type(vk::DescriptorType::STORAGE_IMAGE)
            .image_info(&info);
        // SAFETY: as above.
        unsafe { raw.update_descriptor_sets(&[write], &[]) };
        StorageImageId(index)
    }

    /// Returns a slot to the free list. The caller guarantees no in-flight frame reads it.
    pub(crate) fn release_sampled(&self, id: SampledImageId) {
        self.free_sampled.lock().push(id.0);
    }

    /// Returns a slot to the free list. The caller guarantees no in-flight frame reads it.
    pub(crate) fn release_storage(&self, id: StorageImageId) {
        self.free_storage.lock().push(id.0);
    }

    pub(crate) fn destroy(&mut self, raw: &ash::Device) {
        // SAFETY: called from `Device::drop` after the GPU is idle.
        unsafe {
            for sampler in self.samplers.drain(..) {
                raw.destroy_sampler(sampler, None);
            }
            raw.destroy_descriptor_pool(self.pool, None);
            raw.destroy_descriptor_set_layout(self.layout, None);
        }
    }
}

//! The material table on the GPU (issue #20, D-007): the render layer of every
//! [`forge_core::MaterialTable`] row as a [`GpuMaterial`], which the visibility resolve reads
//! by each instance's material id, and the textures the rows sample ([`TextureSet`]).
//!
//! The resolve shades by class (`MeshletRenderer::resolve`, D-026): the standard pass covers the
//! target and lists the 8×8 tiles that show each other shading class, whose own dispatches
//! then shade them.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_core::material::{MaterialTable, TextureId};
use forge_gpu::{Device, Image, ImageDesc, Result, SampledImageId, vk};

use crate::textures::TextureData;

/// No texture (`TEXTURE_NONE` in `meshlet.slang`).
pub const TEXTURE_NONE: u32 = u32::MAX;

/// Mirrors `Material` in `meshlet.slang` (80 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuMaterial {
    color_a: [f32; 3],
    class_index: u32,
    color_b: [f32; 3],
    cavity: f32,
    emissive: [f32; 3],
    specular_power: f32,
    scatter: [f32; 3],
    specular: f32,
    albedo_texture: u32,
    normal_texture: u32,
    inv_texture_scale: f32,
    normal_strength: f32,
}

const _: () = assert!(std::mem::size_of::<GpuMaterial>() == 80);

/// The textures a world's materials sample, uploaded with their mips and visible to every
/// shader through the bindless set. Released when dropped.
pub struct TextureSet {
    device: Arc<Device>,
    textures: Vec<(Image, SampledImageId)>,
    bytes: u64,
}

impl TextureSet {
    /// An empty set.
    pub fn new(device: &Arc<Device>) -> Self {
        Self {
            device: device.clone(),
            textures: Vec::new(),
            bytes: 0,
        }
    }

    /// Uploads `data` (RGBA8, every level) and returns its id for a material row.
    pub fn add(&mut self, data: &TextureData) -> Result<TextureId> {
        let levels: Vec<&[u8]> = data.levels.iter().map(Vec::as_slice).collect();
        let image = self.device.create_image_with_mips(
            ImageDesc {
                width: data.size,
                height: data.size,
                format: if data.srgb {
                    vk::Format::R8G8B8A8_SRGB
                } else {
                    vk::Format::R8G8B8A8_UNORM
                },
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: levels.len() as u32,
                name: data.name,
            },
            &levels,
        )?;
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        self.bytes += levels.iter().map(|l| l.len() as u64).sum::<u64>();
        self.textures.push((image, sampled));
        Ok(TextureId(self.textures.len() as u32 - 1))
    }

    /// Uploads a layer map for a [`ShadingClass::Layered`] row: `width × height` layer ids,
    /// one byte each, rows top to bottom (`R8_UINT`, one level: the resolve reads texels, it
    /// does not filter them).
    ///
    /// [`ShadingClass::Layered`]: forge_core::material::ShadingClass::Layered
    pub fn add_layer_map(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        layers: &[u8],
    ) -> Result<TextureId> {
        assert_eq!(layers.len(), (width * height) as usize);
        let image = self.device.create_image_with_data(
            ImageDesc {
                width,
                height,
                format: vk::Format::R8_UINT,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name,
            },
            layers,
        )?;
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        self.bytes += layers.len() as u64;
        self.textures.push((image, sampled));
        Ok(TextureId(self.textures.len() as u32 - 1))
    }

    /// The bindless index of `id`.
    pub fn sampled(&self, id: TextureId) -> u32 {
        self.textures[id.0 as usize].1.0
    }

    /// Number of textures.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// Bytes of texels uploaded, every level counted.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for TextureSet {
    fn drop(&mut self) {
        for (_, sampled) in &self.textures {
            self.device.release_sampled_image(*sampled);
        }
    }
}

/// The GPU rows of `table`, their textures resolved through `textures`.
pub fn gpu_rows(table: &MaterialTable, textures: Option<&TextureSet>) -> Vec<GpuMaterial> {
    let texture = |id: Option<TextureId>| match (id, textures) {
        (Some(id), Some(set)) => set.sampled(id),
        _ => TEXTURE_NONE,
    };
    table
        .rows()
        .iter()
        .map(|m| {
            let r = &m.render;
            GpuMaterial {
                color_a: r.color_a,
                class_index: r.class.index(),
                color_b: r.color_b,
                cavity: r.cavity,
                emissive: r.emissive,
                specular_power: r.specular_power(),
                scatter: r.scatter,
                specular: r.specular,
                albedo_texture: texture(r.albedo_texture),
                normal_texture: texture(r.normal_texture),
                inv_texture_scale: 1.0 / r.texture_scale.max(1e-3),
                normal_strength: r.normal_strength,
            }
        })
        .collect()
}

/// The per-instance hash `hash_color` in `meshlet.slang` makes of an instance id, bit for
/// bit: the standard class mixes a row's two colours by its first byte.
pub fn instance_hash(id: u32) -> [u8; 3] {
    let mut n = id.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    n ^= n >> 16;
    n = n.wrapping_mul(0x7feb_352d);
    n ^= n >> 15;
    n = n.wrapping_mul(0x846c_a68b);
    n ^= n >> 16;
    [n as u8, (n >> 8) as u8, (n >> 16) as u8]
}

/// Rows the demos share.
pub mod stock {
    use forge_core::material::{Material, MaterialTags, PhysicsLayer, RenderLayer, ShadingClass};

    /// Bare rock, each instance between grey and rust, darker in its hollows.
    pub fn rock() -> Material {
        Material::new(
            "rock",
            RenderLayer {
                color_a: [0.42, 0.40, 0.38],
                color_b: [0.45, 0.33, 0.25],
                cavity: 0.2,
                roughness: RenderLayer::roughness_for_power(14.0),
                specular: 0.06,
                ..RenderLayer::default()
            },
        )
    }

    /// Ice: blue in its hollows, paler on its crests, a sharp highlight and a bluish rim
    /// where it turns away from the sun.
    pub fn ice() -> Material {
        Material {
            physics: PhysicsLayer {
                density: 917.0,
                static_friction: 0.1,
                dynamic_friction: 0.03,
                restitution: 0.1,
            },
            tags: MaterialTags::SLIPPERY,
            ..Material::new(
                "ice",
                RenderLayer {
                    class: ShadingClass::Ice,
                    color_a: [0.50, 0.62, 0.78],
                    color_b: [0.70, 0.82, 0.92],
                    roughness: RenderLayer::roughness_for_power(48.0),
                    specular: 0.5,
                    scatter: [0.05 * 0.6, 0.10 * 0.6, 0.18 * 0.6],
                    ..RenderLayer::default()
                },
            )
        }
    }

    /// The asteroid fields' rule since Phase 0: a fifth of the rocks are ice, by the second
    /// byte of their instance hash (`hash_color(id).y < 0.2` in the shader's terms).
    pub fn is_ice(instance_id: u32) -> bool {
        (f32::from(super::instance_hash(instance_id)[1]) / 255.0) < 0.2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::material::{Material, RenderLayer, ShadingClass};

    #[test]
    fn about_a_fifth_of_the_rocks_are_ice() {
        let ice = (0..10_000).filter(|&id| stock::is_ice(id)).count();
        assert!((1_800..2_200).contains(&ice), "{ice}");
        // The byte rule: 51 / 255 is 0.2 exactly, so byte 50 is ice and byte 51 is not.
        assert!(f32::from(50_u8) / 255.0 < 0.2);
        assert!(f32::from(51_u8) / 255.0 >= 0.2);
    }

    #[test]
    fn rows_keep_their_class_and_exponent() {
        let mut table = MaterialTable::new();
        table.add(Material::new(
            "ice",
            RenderLayer {
                class: ShadingClass::Ice,
                roughness: RenderLayer::roughness_for_power(48.0),
                albedo_texture: Some(TextureId(0)),
                ..RenderLayer::default()
            },
        ));
        let rows = gpu_rows(&table, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].class_index, 1);
        assert_eq!(rows[1].specular_power, 48.0);
        // Without a texture set, a named texture is none rather than a wrong index.
        assert_eq!(rows[1].albedo_texture, TEXTURE_NONE);
        assert_eq!(rows[0].normal_texture, TEXTURE_NONE);
    }
}

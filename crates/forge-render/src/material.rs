//! The material table on the GPU (issue #20, D-007): the render layer of every
//! [`forge_core::MaterialTable`] row as a [`GpuMaterial`], which the visibility resolve reads
//! by each instance's material id, and the textures the rows sample ([`TextureSet`]).
//!
//! The resolve shades by class (`MeshletRenderer::resolve`, D-026): the standard pass covers the
//! target and lists the 8×8 tiles that show each other shading class, whose own dispatches
//! then shade them.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_core::material::{MaterialTable, RenderLayer, TextureId, UvMapping, metal_highlight};
use forge_geom::model::{Model, ModelMaterial, TextureRef};
use forge_gpu::{Device, Image, ImageDesc, Result, SampledImageId, vk};

use crate::dds::CompressedTexture;
use crate::textures::{ImageUse, TextureData, decode_image};

pub mod motion;

/// The mask texels a layered material's presence map gathers per texel (D-095): 8 × 8.
pub const PRESENCE_BLOCK: u32 = 8;

/// A layered material's presence map (D-095, [`forge_core::material::Multilayer`]) from its
/// `masks` (layers 1 on, `width × height` bytes each): per block of about [`PRESENCE_BLOCK`] mask
/// texels (the map's size is the masks' over that, rounded up, and a texel falls in the block of
/// the same share of the map, so the map reads by texture coordinates alone, whatever levels of
/// the masks are held), bit `k` set where layer `k` (0, the base, always) is over zero in the block, then
/// the bits of the blocks round each (a filtered sample reads across a block's edge); each next
/// level, half the size rounded down as Vulkan's levels are, the union of the texels under it
/// (the last row and column of an odd size kept in the last), down to one texel. Its size at
/// level 0 and its levels.
pub fn layer_presence(masks: &[&[u8]], width: u32, height: u32) -> (u32, u32, Vec<Vec<u32>>) {
    assert!(masks.len() < 32, "at most 32 layers");
    let (bw, bh) = (
        width.div_ceil(PRESENCE_BLOCK),
        height.div_ceil(PRESENCE_BLOCK),
    );
    let mut blocks = vec![1u32; (bw * bh) as usize];
    for (k, mask) in masks.iter().enumerate() {
        let bit = 1u32 << (k + 1);
        for y in 0..height {
            for x in 0..width {
                if mask[(y * width + x) as usize] != 0 {
                    let (bx, by) = (x * bw / width, y * bh / height);
                    blocks[(by * bw + bx) as usize] |= bit;
                }
            }
        }
    }
    // The blocks round each: a texel at a block's edge filters with its neighbour's.
    let mut level: Vec<u32> = (0..bh)
        .flat_map(|y| (0..bw).map(move |x| (x, y)))
        .map(|(x, y)| {
            let mut bits = 0;
            for ny in y.saturating_sub(1)..=(y + 1).min(bh - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(bw - 1) {
                    bits |= blocks[(ny * bw + nx) as usize];
                }
            }
            bits
        })
        .collect();
    let mut levels: Vec<Vec<u32>> = Vec::new();
    let (mut w, mut h) = (bw, bh);
    loop {
        levels.push(level.clone());
        if w == 1 && h == 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let below = &levels[levels.len() - 1];
        // Texel x of the next level gathers 2x and 2x + 1, the last one up to the end.
        let span = |x: u32, n: u32, size: u32| {
            (2 * x).min(size - 1)..if x + 1 == n { size } else { 2 * x + 2 }
        };
        level = (0..nh)
            .flat_map(|y| (0..nw).map(move |x| (x, y)))
            .map(|(x, y)| {
                let mut bits = 0;
                for sy in span(y, nh, h) {
                    for sx in span(x, nw, w) {
                        bits |= below[(sy * w + sx) as usize];
                    }
                }
                bits
            })
            .collect();
        (w, h) = (nw, nh);
    }
    (bw, bh, levels)
}

/// `level` (RGBA8, `width × height`) and its box-filtered levels down to one texel.
fn box_levels(level: Vec<u8>, width: u32, height: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![level];
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let src = &levels[levels.len() - 1];
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1));
                        sum += u32::from(src[((sy * w + sx) * 4 + c) as usize]);
                    }
                    next[((y * nw + x) * 4 + c) as usize] = sum.div_ceil(4) as u8;
                }
            }
        }
        levels.push(next);
        (w, h) = (nw, nh);
    }
    levels
}

/// The bytes of a `width` x `height` level in `format`, block-compressed formats by their 4 x 4
/// blocks.
fn level_size(format: vk::Format, width: u32, height: u32) -> u64 {
    use vk::Format as F;
    let blocks = u64::from(width.div_ceil(4)) * u64::from(height.div_ceil(4));
    let texels = u64::from(width) * u64::from(height);
    match format {
        F::BC1_RGB_UNORM_BLOCK
        | F::BC1_RGB_SRGB_BLOCK
        | F::BC1_RGBA_UNORM_BLOCK
        | F::BC1_RGBA_SRGB_BLOCK
        | F::BC4_UNORM_BLOCK
        | F::BC4_SNORM_BLOCK => blocks * 8,
        F::BC2_UNORM_BLOCK
        | F::BC2_SRGB_BLOCK
        | F::BC3_UNORM_BLOCK
        | F::BC3_SRGB_BLOCK
        | F::BC5_UNORM_BLOCK
        | F::BC5_SNORM_BLOCK
        | F::BC6H_UFLOAT_BLOCK
        | F::BC6H_SFLOAT_BLOCK
        | F::BC7_UNORM_BLOCK
        | F::BC7_SRGB_BLOCK => blocks * 16,
        F::R8_UNORM | F::R8_SNORM | F::R8_UINT => texels,
        F::R8G8_UNORM | F::R8G8_SNORM => texels * 2,
        F::R16G16B16A16_SFLOAT | F::R16G16B16A16_UNORM => texels * 8,
        F::R32G32B32A32_SFLOAT => texels * 16,
        _ => texels * 4,
    }
}

/// A layered material's masks as [`TextureSet::add_layer_masks`] uploads them, made on the CPU
/// alone: `masks` of `width × height` texels each (one byte, rows top to bottom), four to an
/// RGBA8 texture (mask `k` in channel `k % 4` of texture `k / 4`), each with its box-filtered
/// levels. A loader makes them on its workers and uploads them together ([`upload_textures`],
/// [`TextureSet::add_uploaded`]).
pub fn layer_mask_textures(
    name: &str,
    width: u32,
    height: u32,
    masks: &[&[u8]],
) -> Vec<TextureData> {
    assert!(
        masks.len() <= 32,
        "{} masks: at most 32 layers",
        masks.len()
    );
    let texels = (width * height) as usize;
    assert!(
        masks.iter().all(|m| m.len() == texels),
        "masks of {width} × {height}"
    );
    masks
        .chunks(4)
        .enumerate()
        .map(|(k, group)| {
            let mut level = vec![0u8; texels * 4];
            for (c, mask) in group.iter().enumerate() {
                for (t, &v) in mask.iter().enumerate() {
                    level[t * 4 + c] = v;
                }
            }
            TextureData {
                name: format!("{name} masks {k}").into(),
                size: width,
                height,
                srgb: false,
                levels: box_levels(level, width, height),
            }
        })
        .collect()
}

/// A layered material's masks (`width × height` bytes each, rows top to bottom, layer 1's first)
/// two a texture as BC5, with every level ([`crate::dds::masks_to_bc5`]; D-105's masks in BC4):
/// half the bytes of [`layer_mask_textures`]' RGBA8. Layer `k`'s mask is channel `(k - 1) % 2` of
/// texture `(k - 1) / 2`; an odd count's last texture has an empty second channel. For a
/// [`forge_core::material::Multilayer`] with `pairs`: at most 20 masks.
pub fn layer_mask_textures_bc5(
    name: &str,
    width: u32,
    height: u32,
    masks: &[&[u8]],
) -> Vec<crate::dds::CompressedTexture> {
    assert!(
        masks.len() <= 20,
        "{} masks: at most 20 in pairs",
        masks.len()
    );
    let empty = vec![0u8; (width * height) as usize];
    masks
        .chunks(2)
        .enumerate()
        .map(|(k, pair)| {
            let second = pair.get(1).copied().unwrap_or(&empty);
            crate::dds::masks_to_bc5(&format!("{name} masks {k}"), width, height, pair[0], second)
        })
        .collect()
}

/// No texture (`TEXTURE_NONE` in `meshlet.slang`).
pub const TEXTURE_NONE: u32 = u32::MAX;

/// Mirrors `Material` in `meshlet.slang` (256 bytes).
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
    reflectance: f32,
    /// Ice: light scattered per metre by its bubbles ([`bubble_scattering`]).
    scattering: f32,
    /// `MATERIAL_*` flags.
    flags: u32,
    /// Layered: the contour's layer under its height plus one, 0 for none
    /// ([`forge_core::material::RenderLayer::contour`]).
    contour: u32,
    /// Its layers over the height, one bit each.
    contour_above: u32,
    /// Its height, object space, and how far it wanders (metres).
    contour_height: f32,
    contour_wander: f32,
    /// Its other layers under the height, one bit each (#128).
    contour_below: u32,
    /// UV-mapped (D-047): the metallic-roughness, occlusion and emissive maps.
    metal_rough_texture: u32,
    occlusion_texture: u32,
    emissive_texture: u32,
    /// Its sampler: `SAMPLER_ANISOTROPIC_WRAP` plus its wrapping.
    sampler: u32,
    /// The UV transform's first row (u') and the metalness.
    uv_u: [f32; 3],
    metallic: f32,
    /// Its second row (v') and the roughness.
    uv_v: [f32; 3],
    roughness: f32,
    occlusion_strength: f32,
    /// Cut out under this base colour alpha ([`MATERIAL_MASKED`]), and the factor's alpha.
    alpha_cutoff: f32,
    alpha: f32,
    /// A layered material with [`MATERIAL_MASK_PAIRS`]: its ninth masks' texture (layers 17 and
    /// 18).
    mask_texture_8: u32,
    /// A layered material's layer: roughness from `[0]` to `[1]`, metalness from `[2]` to `[3]`.
    levels: [f32; 4],
    /// A layered material: its first layer's row, its layers, its presence map's bindless index,
    /// with [`MATERIAL_MASK_PAIRS`] its tenth masks' texture (layers 19 and 20), then its masks'
    /// (a texture per four layers, or per two).
    first_layer: u32,
    layer_count: u32,
    presence_texture: u32,
    mask_texture_9: u32,
    mask_textures: [u32; 8],
    /// Its relief's height map ([`MATERIAL_RELIEF`], D-107), [`TEXTURE_NONE`] without.
    height_texture: u32,
    /// Metres of the object's space from its relief's top to its floor.
    parallax_depth: f32,
    /// The height map's value that lies on the mesh's surface.
    parallax_top: f32,
    /// The share of the sky's and the probes' light its relief's floor loses.
    parallax_cavity: f32,
}

const _: () = assert!(std::mem::size_of::<GpuMaterial>() == 256);

impl GpuMaterial {
    /// What a far impostor drawn with this row keeps of its look (D-075 step 4): the share of the
    /// sun's light through it (foliage), its reflectance, and its highlight's weight and power.
    pub(crate) fn sheen(&self) -> ([f32; 3], f32, f32, f32) {
        let through = if self.flags & MATERIAL_FOLIAGE != 0 {
            self.scatter
        } else {
            [0.0; 3]
        };
        (
            through,
            self.reflectance,
            self.specular,
            self.specular_power,
        )
    }

    /// Its two colours, which each instance mixes by its id (`hash_color` in meshlet.slang).
    pub(crate) fn tones(&self) -> [[f32; 3]; 2] {
        [self.color_a, self.color_b]
    }

    /// Both its colours set to their mean: what an impostor's bake draws (D-075 step 4), each
    /// impostor then taking its own instance's mix.
    pub(crate) fn mean_tone(&mut self) {
        let mean = std::array::from_fn(|k| 0.5 * (self.color_a[k] + self.color_b[k]));
        self.color_a = mean;
        self.color_b = mean;
    }

    /// Its texture fields (D-105's second step): the colour, normal, metal-roughness (or
    /// metalness), occlusion, emissive, presence and height maps, then its masks (with
    /// [`MATERIAL_MASK_PAIRS`], ten), or a microblend row's microblend alone: the mask fields
    /// after the first hold its floats (tiling, contrast, strength, offset;
    /// [`MATERIAL_MICROBLEND`]).
    fn texture_fields(&mut self) -> Vec<&mut u32> {
        let pairs = self.flags & MATERIAL_MASK_PAIRS != 0;
        let masks = if self.flags & MATERIAL_MICROBLEND != 0 {
            &mut self.mask_textures[..1]
        } else {
            &mut self.mask_textures[..]
        };
        let mut fields = vec![
            &mut self.albedo_texture,
            &mut self.normal_texture,
            &mut self.metal_rough_texture,
            &mut self.occlusion_texture,
            &mut self.emissive_texture,
            &mut self.presence_texture,
            &mut self.height_texture,
        ];
        fields.extend(masks.iter_mut());
        if pairs {
            fields.push(&mut self.mask_texture_8);
            fields.push(&mut self.mask_texture_9);
        }
        fields
    }

    /// The bindless slots of the textures it samples, unset ones ([`TEXTURE_NONE`]) left out: a
    /// streamer tells which textures a table refers to.
    pub fn texture_slots(&self) -> Vec<u32> {
        let mut row = *self;
        row.texture_fields()
            .into_iter()
            .map(|f| *f)
            .filter(|&s| s != TEXTURE_NONE)
            .collect()
    }

    /// Its texture slots that are `from` moved to `to` (a streamed texture remade in a new slot,
    /// D-105): whether it had any. A microblend row's floats are never taken for slots.
    pub fn remap_texture(&mut self, from: u32, to: u32) -> bool {
        let mut hit = false;
        for field in self.texture_fields() {
            if *field == from {
                *field = to;
                hit = true;
            }
        }
        hit
    }

    /// Whether its clusters take the cut-outs' raster (#171): cut out or double-sided, or drawn
    /// after the resolve (its fragments all go there).
    pub fn cut_out(&self) -> bool {
        self.flags & (MATERIAL_MASKED | MATERIAL_DOUBLE_SIDED | MATERIAL_DRAWN_AFTER) != 0
    }

    /// Whether it is jelly, which the sun's shadow rays see through (#180, D-051).
    pub fn jelly(&self) -> bool {
        self.class_index == forge_core::material::ShadingClass::Jelly.index()
    }

    /// Whether the rays' query loop must see its triangles: cut out, jelly, or a drawn house's
    /// clear glass, which the shadow rays pass (`masked_rows` in `meshlet.slang`, #307).
    pub fn ray_masked(&self) -> bool {
        self.flags & MATERIAL_MASKED != 0 || self.jelly() || self.clear_glass()
    }

    /// Whether it is the clear glass of a house whose rooms are drawn: glazed, and drawn after
    /// the resolve (D-065's part 4). The probes' rays (#280) and the shadow rays pass it.
    pub fn clear_glass(&self) -> bool {
        self.flags & (MATERIAL_GLAZED | MATERIAL_DRAWN_AFTER)
            == MATERIAL_GLAZED | MATERIAL_DRAWN_AFTER
    }

    /// Whether the rays' query loop keeps a hit on this row wherever it keeps one on `other`: the
    /// fields it reads are the same (`opacity_rows_match` in `meshlet.slang` compares the same,
    /// #333). Rows that differ in their tones alone.
    pub(crate) fn same_ray_alpha(&self, other: &GpuMaterial) -> bool {
        self.flags == other.flags
            && self.class_index == other.class_index
            && self.albedo_texture == other.albedo_texture
            && self.sampler == other.sampler
            && self.uv_u == other.uv_u
            && self.uv_v == other.uv_v
            && self.alpha == other.alpha
            && self.alpha_cutoff == other.alpha_cutoff
    }

    /// How the rays' query loop keeps a hit on this row (`ray_candidate_kept` and
    /// `shadow_candidate_kept` in `meshlet.slang`), for the opacity micromaps (#333).
    pub(crate) fn ray_alpha(&self) -> RayAlpha {
        if self.jelly() || self.clear_glass() || self.flags & MATERIAL_GRATING != 0 {
            return RayAlpha::Loop;
        }
        if self.flags & MATERIAL_MASKED == 0 {
            return RayAlpha::Kept;
        }
        if self.albedo_texture == TEXTURE_NONE {
            return RayAlpha::Constant(self.alpha >= self.alpha_cutoff);
        }
        RayAlpha::Texture {
            texture: self.albedo_texture,
            sampler: self.sampler,
            uv: [self.uv_u, self.uv_v],
            alpha: self.alpha,
            cutoff: self.alpha_cutoff,
        }
    }
}

/// How the rays' query loop keeps a hit on a row ([`GpuMaterial::ray_alpha`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RayAlpha {
    /// Everywhere: not cut out.
    Kept,
    /// By rules the micromaps do not follow: jelly and clear glass, which the shadow rays pass,
    /// and a grating's bars.
    Loop,
    /// Everywhere or nowhere: cut out by its factor alone (no texture), or on a mesh without
    /// UVs.
    Constant(bool),
    /// Where the base colour texture's alpha (at level 0, through its sampler, at the UVs
    /// transformed by `uv`'s rows) times `alpha` reaches `cutoff`.
    Texture {
        texture: u32,
        sampler: u32,
        uv: [[f32; 3]; 2],
        alpha: f32,
        cutoff: f32,
    },
}

/// A texture's alpha at level 0, kept on the CPU for the rays' opacity micromaps (#333).
pub(crate) struct AlphaPlane {
    pub width: u32,
    pub height: u32,
    /// A byte a texel, rows top to bottom.
    pub texels: Vec<u8>,
}

/// What the CPU knows of a texture's alpha ([`TextureSet::alpha`]).
#[derive(Clone)]
pub(crate) enum TextureAlpha {
    /// Nothing: uploaded from elsewhere, or not a colour.
    Unknown,
    /// 1 everywhere.
    Opaque,
    /// Its texels.
    Plane(Arc<AlphaPlane>),
}

/// What the CPU knows of a block-compressed texture's alpha ([`crate::dds::alpha_plane`]):
/// unknown for a format whose alpha it doesn't decode.
fn compressed_alpha(texture: &CompressedTexture) -> TextureAlpha {
    let Some(texels) = crate::dds::alpha_plane(texture) else {
        return TextureAlpha::Unknown;
    };
    if texels.iter().all(|&a| a == 255) {
        TextureAlpha::Opaque
    } else {
        TextureAlpha::Plane(Arc::new(AlphaPlane {
            width: texture.width,
            height: texture.height,
            texels,
        }))
    }
}

/// The alpha of RGBA8 `data`'s level 0.
fn texture_alpha(data: &TextureData) -> TextureAlpha {
    let Some(level) = data.levels.first() else {
        return TextureAlpha::Unknown;
    };
    if level.len() != data.size as usize * data.height as usize * 4 {
        return TextureAlpha::Unknown;
    }
    let (texels, _) = level.as_chunks::<4>();
    if texels.iter().all(|t| t[3] == 255) {
        return TextureAlpha::Opaque;
    }
    TextureAlpha::Plane(Arc::new(AlphaPlane {
        width: data.size,
        height: data.height,
        texels: texels.iter().map(|t| t[3]).collect(),
    }))
}

/// [`GpuMaterial`] flag: the textures are hex-tiled and offset per instance
/// ([`forge_core::material::RenderLayer::hex_tiling`]).
pub const MATERIAL_HEX_TILING: u32 = 1;

/// [`GpuMaterial`] flag: the textures read the mesh's UVs ([`forge_core::material::UvMapping`]).
pub const MATERIAL_UV: u32 = 2;

/// [`GpuMaterial`] flag: cut out where the base colour's alpha falls under `alpha_cutoff`
/// ([`forge_core::material::UvMapping::alpha_cutoff`], #171).
pub const MATERIAL_MASKED: u32 = 4;

/// [`GpuMaterial`] flag: drawn from both sides ([`forge_core::material::RenderLayer::double_sided`]).
pub const MATERIAL_DOUBLE_SIDED: u32 = 8;

/// [`GpuMaterial`] flag: a layered row whose layers come from the frame's planet
/// ([`forge_core::material::RenderLayer::planet_layers`], #220).
pub const MATERIAL_PLANET: u32 = 16;

/// [`GpuMaterial`] flag: the textures and their brightness noise projected from the scene frame
/// ([`forge_core::material::RenderLayer::world_projected`]).
pub const MATERIAL_WORLD_PROJECTED: u32 = 32;

/// [`GpuMaterial`] flag: a window's glass with a room behind it
/// ([`forge_core::material::RenderLayer::interior`]).
pub const MATERIAL_INTERIOR: u32 = 64;

/// [`GpuMaterial`] flag: a grating of upright bars on one card
/// ([`forge_core::material::RenderLayer::grating`]), with [`MATERIAL_MASKED`] and
/// [`MATERIAL_DOUBLE_SIDED`]: `alpha` is the bars' share of their spacing, `uv_u` reads the
/// mesh's `u`.
pub const MATERIAL_GRATING: u32 = 128;

/// [`GpuMaterial`] flag, with [`MATERIAL_INTERIOR`]: a shop behind the glass
/// ([`forge_core::material::RenderLayer::shop`]).
pub const MATERIAL_SHOP: u32 = 256;

/// [`GpuMaterial`] flag: `emissive` in cd/m², converted each frame by the frame's reference
/// illuminance ([`forge_core::material::RenderLayer::emissive_absolute`], #254).
pub const MATERIAL_EMISSIVE_ABSOLUTE: u32 = 512;

/// [`GpuMaterial`] flag: leaf cards ([`forge_core::material::RenderLayer::foliage`]), with
/// [`MATERIAL_MASKED`] and [`MATERIAL_DOUBLE_SIDED`]: their normals never turned towards the
/// viewer, a mix of the two colours per instance.
pub const MATERIAL_FOLIAGE: u32 = 1024;

/// [`GpuMaterial`] flag: a window's glass a probe's ray from its room passes
/// ([`forge_core::material::RenderLayer::glazed`], or `interior`; D-068).
pub const MATERIAL_GLAZED: u32 = 2048;

/// [`GpuMaterial`] flag: a lamp's glowing part ([`forge_core::material::RenderLayer::lamp`],
/// D-065): the probes' rays take none of its glow.
pub const MATERIAL_LAMP: u32 = 4096;

/// [`GpuMaterial`] flag: left out of the visibility pass, drawn after the resolve
/// ([`forge_core::material::RenderLayer::drawn_after`], D-065's part 4): its clusters take the
/// cut-outs' raster, which drops every fragment of it.
pub const MATERIAL_DRAWN_AFTER: u32 = 8192;

/// [`GpuMaterial`]'s flags: a layered material's layer, its roughness and metalness from
/// single-channel maps remapped to its levels ([`forge_core::material::UvMapping::levels`]).
pub const MATERIAL_LEVELS: u32 = 16384;

/// [`GpuMaterial`]'s flags: a layered material (D-095), its layers rows after it.
pub const MATERIAL_MULTILAYER: u32 = 32768;

/// [`GpuMaterial`]'s flags: a layered material's layer with a microblend
/// ([`forge_core::material::Microblend`]), held in its row's mask slots (a layer has no masks of
/// its own): its texture, then its tiles, contrast, normal strength and offset as float bits.
pub const MATERIAL_MICROBLEND: u32 = 65536;

/// [`GpuMaterial`]'s flags: with [`MATERIAL_INTERIOR`], the rooms as pictures of rooms
/// ([`forge_core::material::RoomAtlas`]): the atlas in the colour map's slot, a room's size in
/// `uv_u` and the atlas's rooms across and down in `uv_v`.
pub const MATERIAL_ROOM_ATLAS: u32 = 131072;

/// [`GpuMaterial`]'s flags: with [`MATERIAL_MULTILAYER`], its masks two a texture (BC5,
/// [`forge_core::material::Multilayer::pairs`]): layer `k`'s mask is channel `(k - 1) % 2` of
/// its masks' texture `(k - 1) / 2`, the ninth and tenth in the row's own two words.
pub const MATERIAL_MASK_PAIRS: u32 = 262144;

/// [`GpuMaterial`]'s flags: its height map is marched in the resolve
/// ([`forge_core::material::Parallax`], D-107).
pub const MATERIAL_RELIEF: u32 = 524288;

/// The textures a world's materials sample, uploaded with their mips and visible to every
/// shader through the bindless set. Released when dropped.
pub struct TextureSet {
    device: Arc<Device>,
    textures: Vec<(Image, SampledImageId)>,
    bytes: u64,
    /// Per texture, what the CPU knows of its alpha (the rays' opacity micromaps, #333).
    alpha: Vec<TextureAlpha>,
}

impl TextureSet {
    /// An empty set.
    pub fn new(device: &Arc<Device>) -> Self {
        Self {
            device: device.clone(),
            textures: Vec::new(),
            bytes: 0,
            alpha: Vec::new(),
        }
    }

    /// Per texture, its bindless index, its larger side in texels and its levels' bytes, the
    /// finest first (#385's measures, `meshlet::feedback`).
    pub fn level_bytes(&self) -> Vec<(u32, u32, Vec<u64>)> {
        self.textures
            .iter()
            .map(|(image, id)| {
                let e = image.extent();
                let levels = (0..image.mip_levels())
                    .map(|l| {
                        level_size(
                            image.format(),
                            (e.width >> l).max(1),
                            (e.height >> l).max(1),
                        )
                    })
                    .collect();
                (id.0, e.width.max(e.height), levels)
            })
            .collect()
    }

    /// Per texture, in [`Self::level_bytes`]'s order, whether it is block-compressed (a map D-105
    /// streams; the masks, presence and layer maps stay resident).
    pub fn compressed(&self) -> Vec<bool> {
        self.textures
            .iter()
            .map(|(image, _)| {
                use vk::Format as F;
                matches!(
                    image.format(),
                    F::BC1_RGB_UNORM_BLOCK
                        | F::BC1_RGB_SRGB_BLOCK
                        | F::BC1_RGBA_UNORM_BLOCK
                        | F::BC1_RGBA_SRGB_BLOCK
                        | F::BC2_UNORM_BLOCK
                        | F::BC2_SRGB_BLOCK
                        | F::BC3_UNORM_BLOCK
                        | F::BC3_SRGB_BLOCK
                        | F::BC4_UNORM_BLOCK
                        | F::BC4_SNORM_BLOCK
                        | F::BC5_UNORM_BLOCK
                        | F::BC5_SNORM_BLOCK
                        | F::BC6H_UFLOAT_BLOCK
                        | F::BC6H_SFLOAT_BLOCK
                        | F::BC7_UNORM_BLOCK
                        | F::BC7_SRGB_BLOCK
                )
            })
            .collect()
    }

    /// What the CPU knows of the alpha of the texture of bindless index `sampled`.
    pub(crate) fn alpha(&self, sampled: u32) -> TextureAlpha {
        self.textures
            .iter()
            .position(|t| t.1.0 == sampled)
            .map_or(TextureAlpha::Unknown, |i| self.alpha[i].clone())
    }

    /// Uploads `data` (RGBA8, every level) and returns its id for a material row.
    pub fn add(&mut self, data: &TextureData) -> Result<TextureId> {
        let levels: Vec<&[u8]> = data.levels.iter().map(Vec::as_slice).collect();
        let image = self.device.create_image_with_mips(
            ImageDesc {
                width: data.size,
                height: data.height,
                format: if data.srgb {
                    vk::Format::R8G8B8A8_SRGB
                } else {
                    vk::Format::R8G8B8A8_UNORM
                },
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: levels.len() as u32,
                name: &data.name,
            },
            &levels,
        )?;
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        self.bytes += levels.iter().map(|l| l.len() as u64).sum::<u64>();
        self.textures.push((image, sampled));
        self.alpha.push(texture_alpha(data));
        Ok(TextureId(self.textures.len() as u32 - 1))
    }

    /// Uploads a layered material's masks ([`forge_core::material::Multilayer`], D-095): `masks`
    /// of `width × height` texels each (one byte, rows top to bottom), four to an RGBA8 texture
    /// (mask `k` in channel `k % 4` of texture `k / 4`), each with its box-filtered levels. Their
    /// ids, a texture per four masks (none for none).
    pub fn add_layer_masks(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        masks: &[&[u8]],
    ) -> Result<Vec<TextureId>> {
        layer_mask_textures(name, width, height, masks)
            .iter()
            .map(|data| self.add(data))
            .collect()
    }

    /// Uploads a layered material's presence map ([`layer_presence`]'s levels, `width × height`
    /// at level 0), `R32_UINT`, read by texel.
    pub fn add_presence(
        &mut self,
        name: &str,
        width: u32,
        height: u32,
        levels: &[Vec<u32>],
    ) -> Result<TextureId> {
        let bytes: Vec<&[u8]> = levels.iter().map(|l| bytemuck::cast_slice(l)).collect();
        let image = self.device.create_image_with_mips(
            ImageDesc {
                width,
                height,
                format: vk::Format::R32_UINT,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: bytes.len() as u32,
                name,
            },
            &bytes,
        )?;
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        self.bytes += bytes.iter().map(|l| l.len() as u64).sum::<u64>();
        self.textures.push((image, sampled));
        self.alpha.push(TextureAlpha::Unknown);
        Ok(TextureId(self.textures.len() as u32 - 1))
    }

    /// Uploads presence maps as [`Self::add_presence`] would, each a name, its size at level 0
    /// and its levels, as many to a submission as [`forge_gpu::STAGING_CHUNK`] holds: a loader
    /// with thousands of layered setups waits a few times, not once a map. Their ids, in order.
    pub fn add_presences(
        &mut self,
        maps: &[(&str, u32, u32, &[Vec<u32>])],
    ) -> Result<Vec<TextureId>> {
        let bytes = |levels: &[Vec<u32>]| levels.iter().map(|l| l.len() * 4).sum::<usize>();
        let mut ids = Vec::with_capacity(maps.len());
        let mut start = 0;
        while start < maps.len() {
            // At least one, then as many more as fit.
            let mut end = start + 1;
            let mut total = bytes(maps[start].3);
            while end < maps.len() && total + bytes(maps[end].3) <= forge_gpu::STAGING_CHUNK {
                total += bytes(maps[end].3);
                end += 1;
            }
            let levels: Vec<Vec<&[u8]>> = maps[start..end]
                .iter()
                .map(|m| m.3.iter().map(|l| bytemuck::cast_slice(l)).collect())
                .collect();
            let images: Vec<(ImageDesc<'_>, &[&[u8]])> = maps[start..end]
                .iter()
                .zip(&levels)
                .map(|(&(name, width, height, _), levels)| {
                    (
                        ImageDesc {
                            width,
                            height,
                            format: vk::Format::R32_UINT,
                            usage: vk::ImageUsageFlags::SAMPLED,
                            aspect: vk::ImageAspectFlags::COLOR,
                            mip_levels: levels.len() as u32,
                            name,
                        },
                        levels.as_slice(),
                    )
                })
                .collect();
            let made = self.device.create_images_with_mips(&images)?;
            for (image, levels) in made.into_iter().zip(&levels) {
                let b = levels.iter().map(|l| l.len() as u64).sum();
                ids.push(self.add_uploaded(image, b));
            }
            start = end;
        }
        Ok(ids)
    }

    /// Adds a block-compressed texture [`upload_compressed`] uploaded, of `bytes` over its
    /// levels, with what the CPU knows of its alpha (the rays' opacity micromaps, #333), and
    /// returns its id for a material row. A two-channel (BC5) normal map is sampled through a
    /// view that reads its missing blue as one: (x, y, 1), which the shading normalises, near the
    /// stored normal for the slopes maps hold, with no change to the shaders (the resolve stands
    /// near the size where its ray queries against movers fail, #355).
    pub fn add_compressed(
        &mut self,
        image: Image,
        bytes: u64,
        texture: &CompressedTexture,
    ) -> Result<TextureId> {
        self.add_compressed_with(image, bytes, texture, compressed_alpha(texture))
    }

    /// Adds every texture [`upload_compressed`] uploaded from `textures`, as
    /// [`Self::add_compressed`] adds one, what the CPU knows of their alpha found on all cores (a
    /// loader's thousands of maps decoded one after another took a good part of its upload);
    /// their ids, in order.
    pub fn add_all_compressed(
        &mut self,
        uploaded: Vec<(Image, u64)>,
        textures: &[CompressedTexture],
    ) -> Result<Vec<TextureId>> {
        assert_eq!(
            uploaded.len(),
            textures.len(),
            "add_all_compressed: one uploaded image per texture"
        );
        let mut alphas = vec![TextureAlpha::Unknown; textures.len()];
        forge_core::pack::par_chunks_mut(&mut alphas, 1, 4, |i, out| {
            out[0] = compressed_alpha(&textures[i]);
        });
        uploaded
            .into_iter()
            .zip(textures)
            .zip(alphas)
            .map(|(((image, bytes), texture), alpha)| {
                self.add_compressed_with(image, bytes, texture, alpha)
            })
            .collect()
    }

    /// [`Self::add_compressed`] with its alpha found.
    fn add_compressed_with(
        &mut self,
        image: Image,
        bytes: u64,
        texture: &CompressedTexture,
        alpha: TextureAlpha,
    ) -> Result<TextureId> {
        let image = if texture.format == crate::dds::BlockFormat::Bc5 {
            image.with_view_components(vk::ComponentMapping {
                r: vk::ComponentSwizzle::IDENTITY,
                g: vk::ComponentSwizzle::IDENTITY,
                b: vk::ComponentSwizzle::ONE,
                a: vk::ComponentSwizzle::ONE,
            })?
        } else {
            image
        };
        let id = self.add_uploaded(image, bytes);
        self.alpha[id.0 as usize] = alpha;
        Ok(id)
    }

    /// Uploads every texture of `data` as [`Self::add`] would, as many to a submission as the
    /// staging holds ([`upload_textures`]); their ids, in order. A loader that made its textures on
    /// its workers uploads them so, not one wait a texture.
    pub fn add_all(&mut self, data: &[TextureData]) -> Result<Vec<TextureId>> {
        let uploaded = upload_textures(&self.device, data)?;
        // Their alpha on all cores: a loader's thousands of mask textures scanned one after
        // another took about as long as their upload.
        let mut alphas = vec![TextureAlpha::Unknown; data.len()];
        forge_core::pack::par_chunks_mut(&mut alphas, 1, 4, |i, out| {
            out[0] = texture_alpha(&data[i]);
        });
        Ok(uploaded
            .into_iter()
            .zip(alphas)
            .map(|((image, bytes), alpha)| {
                let id = self.add_uploaded(image, bytes);
                self.alpha[id.0 as usize] = alpha;
                id
            })
            .collect())
    }

    /// Adds a texture [`upload_textures`] uploaded, of `bytes` over its levels, and returns
    /// its id for a material row.
    pub fn add_uploaded(&mut self, image: Image, bytes: u64) -> TextureId {
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        self.bytes += bytes;
        self.textures.push((image, sampled));
        self.alpha.push(TextureAlpha::Unknown);
        TextureId(self.textures.len() as u32 - 1)
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
        self.alpha.push(TextureAlpha::Unknown);
        Ok(TextureId(self.textures.len() as u32 - 1))
    }

    /// The bindless index of `id`.
    pub fn sampled(&self, id: TextureId) -> u32 {
        self.textures[id.0 as usize].1.0
    }

    /// Remakes texture `id` with `levels`, finest first, the finest `width × height` in `format`
    /// (D-105's first step: a streamed texture grows or shrinks so). The new image is uploaded
    /// whole into a new slot. The old one is left untouched, since frames in flight on either
    /// queue may still sample it; it comes back with its slot to retire once none may
    /// ([`RetiredTexture`], `Frames::destroy_later`). Rows that refer to the texture take its new
    /// slot ([`Self::sampled`]). A two-channel (BC5) image is viewed as [`Self::add_compressed`]
    /// views one. What the CPU knows of its alpha stays: its levels are the same picture's.
    pub fn remake(
        &mut self,
        id: TextureId,
        width: u32,
        height: u32,
        format: vk::Format,
        levels: &[&[u8]],
    ) -> Result<RetiredTexture> {
        // An image takes at least one level: none would register one of undefined contents.
        if levels.is_empty() {
            return Err(forge_gpu::GpuError::Unsupported(
                "a texture remade with no levels".into(),
            ));
        }
        let image = self.device.create_image_with_mips(
            ImageDesc {
                width,
                height,
                format,
                usage: vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: levels.len() as u32,
                name: "streamed texture",
            },
            levels,
        )?;
        let image = if format == vk::Format::BC5_UNORM_BLOCK {
            image.with_view_components(vk::ComponentMapping {
                r: vk::ComponentSwizzle::IDENTITY,
                g: vk::ComponentSwizzle::IDENTITY,
                b: vk::ComponentSwizzle::ONE,
                a: vk::ComponentSwizzle::ONE,
            })?
        } else {
            image
        };
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let (old, slot) = std::mem::replace(&mut self.textures[id.0 as usize], (image, sampled));
        self.bytes = self.bytes.saturating_sub(image_bytes(&old))
            + levels.iter().map(|l| l.len() as u64).sum::<u64>();
        Ok(RetiredTexture {
            device: self.device.clone(),
            image: Some(old),
            slot,
        })
    }

    /// [`Self::remake`] with `texture`'s levels from level `top` down: the coarser ones a
    /// streamed texture keeps, or all of them back.
    pub fn remake_compressed(
        &mut self,
        id: TextureId,
        texture: &CompressedTexture,
        top: usize,
    ) -> Result<RetiredTexture> {
        let top = top.min(texture.levels.len().saturating_sub(1));
        let levels: Vec<&[u8]> = texture.levels[top..].iter().map(Vec::as_slice).collect();
        self.remake(
            id,
            (texture.width >> top).max(1),
            (texture.height >> top).max(1),
            texture.format.vk_format(texture.srgb),
            &levels,
        )
    }

    /// [`Self::remake`] with an image [`upload_compressed`] made of `texture` (its `bytes`): several
    /// remade together wait once for all of them.
    pub fn replace_compressed(
        &mut self,
        id: TextureId,
        image: Image,
        bytes: u64,
        texture: &CompressedTexture,
    ) -> Result<RetiredTexture> {
        let image = if texture.format == crate::dds::BlockFormat::Bc5 {
            image.with_view_components(vk::ComponentMapping {
                r: vk::ComponentSwizzle::IDENTITY,
                g: vk::ComponentSwizzle::IDENTITY,
                b: vk::ComponentSwizzle::ONE,
                a: vk::ComponentSwizzle::ONE,
            })?
        } else {
            image
        };
        let sampled = self
            .device
            .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let (old, slot) = std::mem::replace(&mut self.textures[id.0 as usize], (image, sampled));
        self.bytes = self.bytes.saturating_sub(image_bytes(&old)) + bytes;
        Ok(RetiredTexture {
            device: self.device.clone(),
            image: Some(old),
            slot,
        })
    }

    /// The device its images are on.
    pub fn device(&self) -> &Arc<Device> {
        &self.device
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

/// A texture's image and slot that [`TextureSet::remake`] replaced: dropped (through
/// `Frames::destroy_later`, once no frame in flight may sample them), the image goes and the slot
/// is freed.
#[must_use = "dropped now, it frees the image and its slot while frames in flight may still               sample them: give it to `Frames::destroy_later`"]
pub struct RetiredTexture {
    device: Arc<Device>,
    image: Option<Image>,
    slot: SampledImageId,
}

impl RetiredTexture {
    /// The bindless index it held.
    pub fn slot(&self) -> u32 {
        self.slot.0
    }
}

impl Drop for RetiredTexture {
    fn drop(&mut self) {
        self.image = None;
        self.device.release_sampled_image(self.slot);
    }
}

/// The bytes of `image`'s levels.
fn image_bytes(image: &Image) -> u64 {
    let e = image.extent();
    (0..image.mip_levels())
        .map(|l| {
            level_size(
                image.format(),
                (e.width >> l).max(1),
                (e.height >> l).max(1),
            )
        })
        .sum()
}

/// Uploads every texture of `data` (RGBA8, every level) as [`TextureSet::add`] would, as many
/// to a submission as [`forge_gpu::STAGING_CHUNK`] holds: a worker adding hundreds of the
/// planet's tiles' normal maps while frames run waits a few times, not once a map (#220).
/// Each image with its bytes, for [`TextureSet::add_uploaded`].
pub fn upload_textures(device: &Arc<Device>, data: &[TextureData]) -> Result<Vec<(Image, u64)>> {
    let bytes = |d: &TextureData| d.levels.iter().map(Vec::len).sum::<usize>();
    let mut uploaded = Vec::with_capacity(data.len());
    let mut start = 0;
    while start < data.len() {
        // At least one, then as many more as fit.
        let mut end = start + 1;
        let mut total = bytes(&data[start]);
        while end < data.len() && total + bytes(&data[end]) <= forge_gpu::STAGING_CHUNK {
            total += bytes(&data[end]);
            end += 1;
        }
        let levels: Vec<Vec<&[u8]>> = data[start..end]
            .iter()
            .map(|d| d.levels.iter().map(Vec::as_slice).collect())
            .collect();
        let images: Vec<(ImageDesc<'_>, &[&[u8]])> = data[start..end]
            .iter()
            .zip(&levels)
            .map(|(d, levels)| {
                (
                    ImageDesc {
                        width: d.size,
                        height: d.height,
                        format: if d.srgb {
                            vk::Format::R8G8B8A8_SRGB
                        } else {
                            vk::Format::R8G8B8A8_UNORM
                        },
                        usage: vk::ImageUsageFlags::SAMPLED,
                        aspect: vk::ImageAspectFlags::COLOR,
                        mip_levels: levels.len() as u32,
                        name: &d.name,
                    },
                    levels.as_slice(),
                )
            })
            .collect();
        let made = device.create_images_with_mips(&images)?;
        uploaded.extend(
            made.into_iter()
                .zip(&data[start..end])
                .map(|(image, d)| (image, bytes(d) as u64)),
        );
        start = end;
    }
    Ok(uploaded)
}

/// Uploads every block-compressed texture of `data` with its levels as they are, as many to a
/// submission as [`forge_gpu::STAGING_CHUNK`] holds ([`upload_textures`]'s way). Each image
/// with its bytes, for [`TextureSet::add_compressed`].
pub fn upload_compressed(
    device: &Arc<Device>,
    data: &[CompressedTexture],
) -> Result<Vec<(Image, u64)>> {
    let bytes = |d: &CompressedTexture| d.levels.iter().map(Vec::len).sum::<usize>();
    let mut uploaded = Vec::with_capacity(data.len());
    let mut start = 0;
    while start < data.len() {
        let mut end = start + 1;
        let mut total = bytes(&data[start]);
        while end < data.len() && total + bytes(&data[end]) <= forge_gpu::STAGING_CHUNK {
            total += bytes(&data[end]);
            end += 1;
        }
        let levels: Vec<Vec<&[u8]>> = data[start..end]
            .iter()
            .map(|d| d.levels.iter().map(Vec::as_slice).collect())
            .collect();
        let images: Vec<(ImageDesc<'_>, &[&[u8]])> = data[start..end]
            .iter()
            .zip(&levels)
            .map(|(d, levels)| {
                (
                    ImageDesc {
                        width: d.width,
                        height: d.height,
                        format: d.format.vk_format(d.srgb),
                        usage: vk::ImageUsageFlags::SAMPLED,
                        aspect: vk::ImageAspectFlags::COLOR,
                        mip_levels: levels.len() as u32,
                        name: &d.name,
                    },
                    levels.as_slice(),
                )
            })
            .collect();
        let made = device.create_images_with_mips(&images)?;
        uploaded.extend(
            made.into_iter()
                .zip(&data[start..end])
                .map(|(image, d)| (image, bytes(d) as u64)),
        );
        start = end;
    }
    Ok(uploaded)
}

/// The maps a model's material takes from outside its file (D-083: the Bistro's DDS files,
/// named beside a glTF that holds none), as [`TextureSet`] ids.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExternalMaps {
    /// Base colour, sRGB, its alpha the opacity.
    pub base_color: Option<TextureId>,
    /// Roughness in green, metalness in blue (glTF's metallic-roughness map).
    pub metal_rough: Option<TextureId>,
    /// Occlusion of the indirect light in red (glTF's occlusion map; often the same image).
    pub occlusion: Option<TextureId>,
    /// A tangent-space normal map, +y up the image (glTF's; a DirectX map turned over first,
    /// [`crate::dds::flip_green`]).
    pub normal: Option<TextureId>,
    /// Emission, sRGB colour.
    pub emissive: Option<TextureId>,
    /// The emission's luminance at full colour, cd/m² (with `emissive`).
    pub emissive_luminance: f32,
    /// Cut out where the base colour's alpha falls under this.
    pub alpha_cutoff: Option<f32>,
}

/// The row of material `m` drawn with `maps` (on a mesh with UVs): its factors as the file
/// gives them, but 1 for each the maps carry (glTF multiplies a map by its factor, and an
/// exporter that kept no image wrote its own default), its UVs as they are, repeated.
pub fn external_layer(m: &ModelMaterial, maps: ExternalMaps) -> RenderLayer {
    let color = if maps.base_color.is_some() {
        [1.0; 3]
    } else {
        [m.base_color[0], m.base_color[1], m.base_color[2]]
    };
    let orm = maps.metal_rough.is_some();
    let emissive = maps
        .emissive
        .map_or(m.emissive, |_| [maps.emissive_luminance; 3]);
    RenderLayer {
        color_a: color,
        color_b: color,
        roughness: if orm {
            1.0
        } else {
            m.roughness.clamp(0.05, 1.0)
        },
        // A metal's highlight weight is added per pixel by its metalness.
        specular: 0.05,
        emissive,
        emissive_absolute: maps.emissive.is_some(),
        double_sided: m.double_sided,
        albedo_texture: maps.base_color,
        normal_texture: maps.normal,
        normal_strength: m.normal_scale,
        uv: Some(UvMapping {
            metallic: if orm { 1.0 } else { m.metallic },
            metallic_roughness_texture: maps.metal_rough,
            occlusion_texture: maps.occlusion,
            occlusion_strength: 1.0,
            emissive_texture: maps.emissive,
            alpha_cutoff: maps.alpha_cutoff,
            alpha: 1.0,
            ..UvMapping::default()
        }),
        ..RenderLayer::default()
    }
}

/// A model's materials as rows (#138, D-047), its images decoded into a [`TextureSet`] once
/// each for the use its materials make of them.
pub struct ModelTextures<'a> {
    model: &'a Model,
    decoded: HashMap<(usize, ImageUse), Option<TextureId>>,
}

impl<'a> ModelTextures<'a> {
    /// For `model`, called `name` in the logs: what it asks for that Forge does not draw yet
    /// is logged here, once.
    pub fn new(model: &'a Model, name: &str) -> Self {
        for line in &model.unsupported {
            tracing::warn!(model = name, "{line}");
        }
        Self {
            model,
            decoded: HashMap::new(),
        }
    }

    /// The row of material `m` of a mesh that has texture coordinates when `uvs`: its base
    /// colour, roughness and emission, a highlight that grows with its metalness and, when it
    /// has textures and the mesh UVs, its maps as the file sets them ([`UvMapping`]). Without
    /// textures, or on a mesh without UVs, the factors alone (flat colours).
    pub fn layer(
        &mut self,
        m: &ModelMaterial,
        uvs: bool,
        textures: &mut TextureSet,
    ) -> RenderLayer {
        let color = [m.base_color[0], m.base_color[1], m.base_color[2]];
        let mut layer = RenderLayer {
            color_a: color,
            color_b: color,
            roughness: m.roughness.clamp(0.05, 1.0),
            emissive: m.emissive,
            double_sided: m.double_sided,
            ..RenderLayer::default()
        };
        // A dielectric's highlight weighs 0.05, a metal's by its reflectance and its lobe.
        let reflectance = color[0].max(color[1]).max(color[2]);
        let metal = metal_highlight(reflectance, layer.specular_power());
        layer.specular = 0.05 + (metal - 0.05) * m.metallic;
        let mapped = [
            m.base_color_texture,
            m.metallic_roughness_texture,
            m.normal_texture,
            m.occlusion_texture,
            m.emissive_texture,
        ];
        let Some(first) = mapped.iter().flatten().next().copied() else {
            return layer;
        };
        if !uvs {
            return layer;
        }
        let mut texture = |t: Option<TextureRef>, usage: ImageUse| {
            t.and_then(|t| self.decode(t.image, usage, textures))
        };
        layer.albedo_texture = texture(m.base_color_texture, ImageUse::Color);
        layer.normal_texture = texture(m.normal_texture, ImageUse::Normal);
        layer.normal_strength = m.normal_scale;
        // A metal's highlight weight is added per pixel by its metalness.
        layer.specular = 0.05;
        layer.uv = Some(UvMapping {
            transform: first.transform,
            wrap: first.wrap,
            metallic: m.metallic,
            metallic_roughness_texture: texture(m.metallic_roughness_texture, ImageUse::Data),
            occlusion_texture: texture(m.occlusion_texture, ImageUse::Data),
            occlusion_strength: m.occlusion_strength,
            emissive_texture: texture(m.emissive_texture, ImageUse::Color),
            alpha_cutoff: m.alpha_cutoff,
            alpha: m.base_color[3],
            ..UvMapping::default()
        });
        layer
    }

    /// Image `image` decoded for `usage` and uploaded once; `None` when it does not decode
    /// (logged; the material then draws its factor).
    fn decode(
        &mut self,
        image: usize,
        usage: ImageUse,
        textures: &mut TextureSet,
    ) -> Option<TextureId> {
        let model = self.model;
        *self.decoded.entry((image, usage)).or_insert_with(|| {
            let source = model.images.get(image)?;
            let name = if source.name.is_empty() {
                format!("model image {image}")
            } else {
                source.name.clone()
            };
            let data = match decode_image(&name, &source.bytes, usage) {
                Ok(data) => data,
                Err(e) => {
                    tracing::warn!(
                        image = name,
                        mime = source.mime,
                        "a model's image does not decode: {e}"
                    );
                    return None;
                }
            };
            match textures.add(&data) {
                Ok(id) => Some(id),
                Err(e) => {
                    tracing::warn!(image = name, "a model's image does not upload: {e}");
                    None
                }
            }
        })
    }
}

/// Radius of the air bubbles in ice, metres: lake and glacier ice hold bubbles of a tenth of a
/// millimetre to a few millimetres (issue #61).
pub const BUBBLE_RADIUS: f32 = 0.001;

/// Light scattered per metre by ice holding `bubbles` of its volume in air bubbles of
/// [`BUBBLE_RADIUS`]: bubbles far larger than the wavelength block twice their cross-section
/// (the extinction paradox), so the coefficient is `2 · πr² · n = 1.5 · bubbles / r`
/// with `n = bubbles / (4/3 πr³)` bubbles per cubic metre.
pub fn bubble_scattering(bubbles: f32) -> f32 {
    1.5 * bubbles.max(0.0) / BUBBLE_RADIUS
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
        .enumerate()
        .map(|(row, m)| {
            let r = &m.render;
            // A window's pictures of rooms: only behind glass, and they take the UVs' slots.
            let atlas = r.room_atlas.filter(|_| r.interior);
            let mapping = r.uv.filter(|_| atlas.is_none());
            let uv = mapping.unwrap_or_default();
            // A layered material whose layers lie inside the table (else drawn as its own colours).
            let rows = table.rows().len();
            let multilayer = r.multilayer.filter(|m| {
                r.uv.is_some()
                    && m.layers >= 1
                    && m.layers <= 32
                    && row + m.offset.max(1) as usize + m.layers as usize <= rows
            });
            GpuMaterial {
                color_a: r.color_a,
                class_index: r.class.index(),
                color_b: r.color_b,
                cavity: r.cavity,
                emissive: r.emissive,
                specular_power: r.specular_power(),
                scatter: r.scatter,
                specular: r.specular,
                albedo_texture: texture(atlas.map(|a| a.texture).or(r.albedo_texture)),
                normal_texture: texture(r.normal_texture),
                inv_texture_scale: 1.0 / r.texture_scale.max(1e-3),
                normal_strength: r.normal_strength,
                reflectance: r.reflectance,
                // Leaf cards take the ice's field for their undersides (#346, `leaf_underside`).
                scattering: if r.foliage {
                    r.underside
                } else {
                    bubble_scattering(r.bubbles)
                },
                flags: if r.hex_tiling { MATERIAL_HEX_TILING } else { 0 }
                    | if mapping.is_some() { MATERIAL_UV } else { 0 }
                    | if atlas.is_some() {
                        MATERIAL_ROOM_ATLAS
                    } else {
                        0
                    }
                    | if uv.alpha_cutoff.is_some() {
                        MATERIAL_MASKED
                    } else {
                        0
                    }
                    | if r.double_sided {
                        MATERIAL_DOUBLE_SIDED
                    } else {
                        0
                    }
                    | if r.planet_layers { MATERIAL_PLANET } else { 0 }
                    | if r.world_projected {
                        MATERIAL_WORLD_PROJECTED
                    } else {
                        0
                    }
                    | if r.interior { MATERIAL_INTERIOR } else { 0 }
                    | if r.interior && r.shop {
                        MATERIAL_SHOP
                    } else {
                        0
                    }
                    | if r.interior || r.glazed {
                        MATERIAL_GLAZED
                    } else {
                        0
                    }
                    | if r.emissive_absolute {
                        MATERIAL_EMISSIVE_ABSOLUTE
                    } else {
                        0
                    }
                    | if r.lamp { MATERIAL_LAMP } else { 0 }
                    | if r.drawn_after {
                        MATERIAL_DRAWN_AFTER
                    } else {
                        0
                    }
                    | if r.grating.is_some() {
                        MATERIAL_GRATING | MATERIAL_MASKED | MATERIAL_DOUBLE_SIDED
                    } else {
                        0
                    }
                    | if r.foliage && uv.alpha_cutoff.is_some() {
                        MATERIAL_FOLIAGE | MATERIAL_DOUBLE_SIDED
                    } else {
                        0
                    }
                    | if uv.levels.is_some() {
                        MATERIAL_LEVELS
                    } else {
                        0
                    }
                    | if multilayer.is_some() {
                        MATERIAL_MULTILAYER
                    } else {
                        0
                    }
                    | if multilayer.is_some_and(|m| m.pairs) {
                        MATERIAL_MASK_PAIRS
                    } else {
                        0
                    }
                    | if multilayer.is_none() && uv.microblend.is_some() {
                        MATERIAL_MICROBLEND
                    } else {
                        0
                    }
                    | if r.parallax.is_some() {
                        MATERIAL_RELIEF
                    } else {
                        0
                    },
                contour: r.contour.map_or(0, |c| u32::from(c.below) + 1),
                contour_above: r.contour.map_or(0, |c| c.above),
                contour_height: r.contour.map_or(0.0, |c| c.height),
                contour_wander: r.contour.map_or(0.0, |c| c.wander),
                contour_below: r.contour.map_or(0, |c| c.others),
                metal_rough_texture: texture(uv.metallic_roughness_texture),
                // A layered material's layer reads its metalness map in the occlusion's place.
                occlusion_texture: texture(if uv.levels.is_some() {
                    uv.metalness_texture
                } else {
                    uv.occlusion_texture
                }),
                emissive_texture: texture(uv.emissive_texture),
                sampler: forge_gpu::SamplerKind::AnisotropicWrap as u32
                    + 3 * uv.wrap[0].index()
                    + uv.wrap[1].index(),
                // A window's rooms: their size and their atlas's shape, read by `interior_atlas`.
                uv_u: atlas.map_or([uv.transform[0], uv.transform[1], uv.transform[2]], |a| {
                    a.room.map(|v| v.max(1e-3))
                }),
                metallic: uv.metallic,
                uv_v: atlas.map_or([uv.transform[3], uv.transform[4], uv.transform[5]], |a| {
                    [
                        a.cells[0].round().max(1.0),
                        a.cells[1].round().max(1.0),
                        0.0,
                    ]
                }),
                roughness: r.roughness,
                occlusion_strength: uv.occlusion_strength,
                alpha_cutoff: uv.alpha_cutoff.unwrap_or(0.0),
                alpha: r.grating.unwrap_or(uv.alpha),
                mask_texture_8: texture(multilayer.and_then(|m| m.masks[8])),
                levels: uv.levels.unwrap_or([0.0, 1.0, 0.0, 1.0]),
                first_layer: multilayer.map_or(0, |m| row as u32 + m.offset.max(1)),
                layer_count: multilayer.map_or(0, |m| m.layers),
                presence_texture: texture(multilayer.map(|m| m.presence)),
                mask_texture_9: texture(multilayer.and_then(|m| m.masks[9])),
                mask_textures: match (multilayer, uv.microblend) {
                    (None, Some(mb)) => [
                        texture(Some(mb.texture)),
                        mb.tiles.to_bits(),
                        mb.contrast.to_bits(),
                        mb.normal_strength.to_bits(),
                        mb.offset[0].to_bits(),
                        mb.offset[1].to_bits(),
                        0,
                        0,
                    ],
                    _ => std::array::from_fn(|k| texture(multilayer.and_then(|m| m.masks[k]))),
                },
                height_texture: texture(r.parallax.map(|p| p.texture)),
                parallax_depth: r.parallax.map_or(0.0, |p| p.depth.max(0.0)),
                parallax_top: r.parallax.map_or(1.0, |p| p.top.max(1e-3)),
                parallax_cavity: r.parallax.map_or(0.0, |p| p.cavity.clamp(0.0, 1.0)),
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

    /// [`ice`] holding `bubbles` of its volume in air (issue #61), named `name`: whiter and
    /// less translucent as they grow, and lighter.
    pub fn bubbly_ice(name: &str, bubbles: f32) -> Material {
        let mut ice = ice();
        ice.name = name.to_owned();
        ice.render.bubbles = bubbles;
        ice.physics.density *= 1.0 - bubbles;
        ice
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

    /// A GPU test (`FORGE_GPU_TESTS=1`): textures added together know their alpha as textures
    /// added one at a time do: opaque, a plane, or unknown.
    /// Three masks in pairs: two BC5 textures named in order, the second's green channel empty.
    #[test]
    fn three_masks_go_into_two_bc5_textures() {
        let (w, h) = (8_u32, 8_u32);
        let masks = [vec![255_u8; 64], vec![0_u8; 64], vec![128_u8; 64]];
        let refs: Vec<&[u8]> = masks.iter().map(Vec::as_slice).collect();
        let t = layer_mask_textures_bc5("setup", w, h, &refs);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].name, "setup masks 0");
        assert_eq!(t[1].name, "setup masks 1");
        assert!(t.iter().all(|t| t.format == crate::dds::BlockFormat::Bc5));
        // 8 x 8 down to 1 x 1: four levels; level 0 four blocks of 16 bytes.
        assert_eq!(t[1].levels.len(), 4);
        assert_eq!(t[1].levels[0].len(), 4 * 16);
        // A block's channel (BC4): its two endpoints first; a flat 0 has both at 0.
        let green = &t[1].levels[0][8..16];
        assert_eq!((green[0], green[1]), (0, 0));
    }

    #[test]
    fn textures_added_together_know_their_alpha() {
        let Some(gpu) = forge_gpu::Headless::for_test(forge_gpu::headless::shaders_of(env!(
            "CARGO_MANIFEST_DIR"
        ))) else {
            return;
        };
        let rgba = |alpha: fn(usize) -> u8| TextureData {
            name: "alpha".into(),
            size: 4,
            height: 4,
            srgb: true,
            levels: vec![
                (0..16).flat_map(|t| [9, 9, 9, alpha(t)]).collect(),
                vec![9; 16],
                vec![9; 4],
            ],
        };
        let data: Vec<TextureData> = (0..40)
            .map(|k| {
                if k % 3 == 0 {
                    rgba(|_| 255)
                } else {
                    rgba(|t| (t * 16) as u8)
                }
            })
            .collect();
        let mut one = TextureSet::new(&gpu.device);
        let mut all = TextureSet::new(&gpu.device);
        for d in &data {
            one.add(d).unwrap();
        }
        all.add_all(&data).unwrap();
        let kind = |a: &TextureAlpha| match a {
            TextureAlpha::Unknown => (0, Vec::new()),
            TextureAlpha::Opaque => (1, Vec::new()),
            TextureAlpha::Plane(p) => (2, p.texels.clone()),
        };
        assert_eq!(
            one.alpha.iter().map(kind).collect::<Vec<_>>(),
            all.alpha.iter().map(kind).collect::<Vec<_>>()
        );
        assert!(
            all.alpha
                .iter()
                .any(|a| matches!(a, TextureAlpha::Plane(_)))
        );
        assert!(all.alpha.iter().any(|a| matches!(a, TextureAlpha::Opaque)));
    }

    /// A row's texture slots, field by field: a plain row's maps, a layered row's presence map
    /// and masks, a microblend row's microblend but never its floats, even one whose bits are the
    /// slot moved.
    #[test]
    fn a_rows_texture_slots_are_moved_field_by_field() {
        let none = |flags: u32| GpuMaterial {
            flags,
            albedo_texture: TEXTURE_NONE,
            normal_texture: TEXTURE_NONE,
            metal_rough_texture: TEXTURE_NONE,
            occlusion_texture: TEXTURE_NONE,
            emissive_texture: TEXTURE_NONE,
            presence_texture: TEXTURE_NONE,
            height_texture: TEXTURE_NONE,
            mask_textures: [TEXTURE_NONE; 8],
            ..bytemuck::Zeroable::zeroed()
        };
        let mut plain = none(MATERIAL_UV);
        plain.albedo_texture = 5;
        plain.normal_texture = 7;
        plain.emissive_texture = 5;
        assert_eq!(plain.texture_slots(), vec![5, 7, 5]);
        assert!(plain.remap_texture(5, 9));
        assert_eq!(plain.texture_slots(), vec![9, 7, 9]);
        assert!(!plain.remap_texture(5, 9));

        let mut layered = none(MATERIAL_UV);
        layered.presence_texture = 12;
        layered.mask_textures[0] = 10;
        layered.mask_textures[1] = 11;
        assert!(layered.remap_texture(11, 20));
        assert_eq!(layered.texture_slots(), vec![12, 10, 20]);
        // Without pairs the row's two words are not slots: 0 in them is no texture 0.
        layered.mask_texture_8 = 0;
        assert_eq!(layered.texture_slots(), vec![12, 10, 20]);

        // In pairs (BC5), the ninth and tenth masks' textures are the row's own two words.
        let mut paired = none(MATERIAL_UV | MATERIAL_MULTILAYER | MATERIAL_MASK_PAIRS);
        paired.presence_texture = 12;
        paired.mask_textures[0] = 10;
        paired.mask_texture_8 = 18;
        paired.mask_texture_9 = TEXTURE_NONE;
        assert_eq!(paired.texture_slots(), vec![12, 10, 18]);
        assert!(paired.remap_texture(18, 28));
        assert_eq!(paired.texture_slots(), vec![12, 10, 28]);

        let tiles = 4.0f32;
        let mut microblend = none(MATERIAL_UV | MATERIAL_MICROBLEND);
        microblend.mask_textures = [
            30,
            tiles.to_bits(),
            0.5f32.to_bits(),
            1.0f32.to_bits(),
            0,
            0,
            0,
            0,
        ];
        assert_eq!(microblend.texture_slots(), vec![30]);
        // A slot equal to the tiling's bits: the tiling stays.
        assert!(!microblend.remap_texture(tiles.to_bits(), 99));
        assert_eq!(f32::from_bits(microblend.mask_textures[1]), tiles);
        assert!(microblend.remap_texture(30, 40));
        assert_eq!(microblend.mask_textures[0], 40);
    }

    /// A GPU test (`FORGE_GPU_TESTS=1`, D-105's first step): a texture remade with fewer levels
    /// takes a new slot and counts only its levels; the slot it left is freed when its retiree
    /// drops; remade with all its levels again, it counts what it did at first.
    #[test]
    fn a_remade_texture_takes_a_new_slot_and_retires_the_old() {
        let Some(gpu) = forge_gpu::Headless::for_test(forge_gpu::headless::shaders_of(env!(
            "CARGO_MANIFEST_DIR"
        ))) else {
            return;
        };
        let mut set = TextureSet::new(&gpu.device);
        let size = |k: u32| (16u32 >> k).max(1);
        let data = TextureData {
            name: "streamed".into(),
            size: 16,
            height: 16,
            srgb: true,
            levels: (0..5)
                .map(|k| vec![(k * 40) as u8; (size(k) * size(k) * 4) as usize])
                .collect(),
        };
        let id = set.add(&data).unwrap();
        let other = set.add(&data).unwrap();
        let (whole, first) = (set.bytes(), set.sampled(id));
        let tail: Vec<&[u8]> = data.levels[2..].iter().map(Vec::as_slice).collect();
        let retired = set
            .remake(id, 4, 4, vk::Format::R8G8B8A8_SRGB, &tail)
            .unwrap();
        assert_eq!(retired.slot(), first);
        assert_ne!(set.sampled(id), first);
        assert_ne!(set.sampled(id), set.sampled(other));
        assert_eq!(set.bytes(), whole - 16 * 16 * 4 - 8 * 8 * 4);
        assert_eq!(set.textures[id.0 as usize].0.mip_levels(), 3);
        assert_eq!(set.textures[id.0 as usize].0.extent().width, 4);
        drop(retired);
        // No levels: refused, the texture as it was.
        assert!(
            set.remake(id, 4, 4, vk::Format::R8G8B8A8_SRGB, &[])
                .is_err()
        );
        assert_eq!(set.textures[id.0 as usize].0.mip_levels(), 3);
        let all: Vec<&[u8]> = data.levels.iter().map(Vec::as_slice).collect();
        let retired = set
            .remake(id, 16, 16, vk::Format::R8G8B8A8_SRGB, &all)
            .unwrap();
        assert_eq!(set.bytes(), whole);
        assert_eq!(set.textures[id.0 as usize].0.mip_levels(), 5);
        drop(retired);
        // A block-compressed one, from its top level down.
        let bc = CompressedTexture {
            name: "bc".into(),
            width: 16,
            height: 16,
            format: crate::dds::BlockFormat::Bc1,
            srgb: true,
            levels: (0..5)
                .map(|k| vec![0u8; (size(k).div_ceil(4) * size(k).div_ceil(4) * 8) as usize])
                .collect(),
        };
        let image = gpu
            .device
            .create_image_with_mips(
                ImageDesc {
                    width: 16,
                    height: 16,
                    format: vk::Format::BC1_RGBA_SRGB_BLOCK,
                    usage: vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 5,
                    name: "bc",
                },
                &bc.levels.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            )
            .unwrap();
        let bc_bytes: u64 = bc.levels.iter().map(|l| l.len() as u64).sum();
        let b = set.add_compressed(image, bc_bytes, &bc).unwrap();
        let before = set.bytes();
        let retired = set.remake_compressed(b, &bc, 3).unwrap();
        assert_eq!(set.textures[b.0 as usize].0.mip_levels(), 2);
        // Its levels 3 and 4: a block each.
        assert_eq!(set.bytes(), before - bc_bytes + 2 * 8);
        drop(retired);
    }

    /// A GPU test (`FORGE_GPU_TESTS=1`): presence maps uploaded together come back as ids in
    /// order, their bytes counted as one at a time would count them, over more than one
    /// submission.
    #[test]
    fn presence_maps_upload_together() {
        let Some(gpu) = forge_gpu::Headless::for_test(forge_gpu::headless::shaders_of(env!(
            "CARGO_MANIFEST_DIR"
        ))) else {
            return;
        };
        let mut one = TextureSet::new(&gpu.device);
        let mut all = TextureSet::new(&gpu.device);
        // 600 maps of 128 x 128 at level 0 (85 KiB with their levels): about 50 MiB, four submissions.
        let mask: Vec<u8> = (0..1024 * 1024).map(|i| u8::from(i % 7 == 0)).collect();
        let (w, h, levels) = layer_presence(&[&mask], 1024, 1024);
        let names: Vec<String> = (0..600).map(|k| format!("presence {k}")).collect();
        let maps: Vec<(&str, u32, u32, &[Vec<u32>])> = names
            .iter()
            .map(|n| (n.as_str(), w, h, levels.as_slice()))
            .collect();
        let ids = all.add_presences(&maps).unwrap();
        assert_eq!(ids, (0..600).map(TextureId).collect::<Vec<_>>());
        for (name, w, h, levels) in &maps {
            one.add_presence(name, *w, *h, levels).unwrap();
        }
        assert_eq!(all.bytes(), one.bytes());
        assert!(all.bytes() as usize > forge_gpu::STAGING_CHUNK);
    }

    #[test]
    fn about_a_fifth_of_the_rocks_are_ice() {
        let ice = (0..10_000).filter(|&id| stock::is_ice(id)).count();
        assert!((1_800..2_200).contains(&ice), "{ice}");
        // The byte rule: 51 / 255 is 0.2 exactly, so byte 50 is ice and byte 51 is not.
        assert!(f32::from(50_u8) / 255.0 < 0.2);
        assert!(f32::from(51_u8) / 255.0 >= 0.2);
    }

    #[test]
    fn a_room_atlas_stands_behind_glass_and_takes_the_uvs_slots() {
        let atlas = forge_core::material::RoomAtlas {
            texture: TextureId(0),
            cells: [2.2, 0.0],
            room: [3.0, 0.0, 4.0],
        };
        let mapping = UvMapping {
            alpha_cutoff: Some(0.5),
            ..UvMapping::default()
        };
        let mut table = MaterialTable::new();
        table.add(Material::new(
            "rooms",
            RenderLayer {
                interior: true,
                room_atlas: Some(atlas),
                uv: Some(mapping),
                ..RenderLayer::default()
            },
        ));
        table.add(Material::new(
            "no glass",
            RenderLayer {
                room_atlas: Some(atlas),
                uv: Some(mapping),
                ..RenderLayer::default()
            },
        ));
        let rows = gpu_rows(&table, None);
        let rooms = &rows[1];
        assert_ne!(rooms.flags & MATERIAL_ROOM_ATLAS, 0);
        assert_eq!(rooms.flags & (MATERIAL_UV | MATERIAL_MASKED), 0);
        // Bad data can't divide by nothing in the shader.
        assert_eq!(rooms.uv_u, [3.0, 1e-3, 4.0]);
        assert_eq!(rooms.uv_v, [2.0, 1.0, 0.0]);
        // Without glass it is an ordinary row, as if no atlas were named.
        let plain = &rows[2];
        assert_eq!(plain.flags & MATERIAL_ROOM_ATLAS, 0);
        assert_ne!(plain.flags & MATERIAL_UV, 0);
        assert_ne!(plain.flags & MATERIAL_MASKED, 0);
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

    #[test]
    fn bubbles_scatter_the_light_and_lighten_the_ice() {
        // A thousandth of the volume in bubbles of a millimetre: 1.5 per metre, a mean free
        // path of 67 cm.
        assert_eq!(bubble_scattering(0.0), 0.0);
        assert!((bubble_scattering(1e-3) - 1.5).abs() < 1e-6);
        let white = stock::bubbly_ice("white ice", 3e-3);
        assert_eq!(white.name, "white ice");
        assert_eq!(white.render.class, ShadingClass::Ice);
        assert!((white.physics.density - 917.0 * 0.997).abs() < 1e-3);
        let mut table = MaterialTable::new();
        table.add(stock::ice());
        table.add(white);
        let rows = gpu_rows(&table, None);
        assert_eq!(rows[1].scattering, 0.0);
        assert!((rows[2].scattering - 4.5).abs() < 1e-5);
    }
    /// A layer's mask over zero in one block marks that block and the blocks round it, a layer
    /// nowhere over zero marks none, the base layer is everywhere, and each coarser level holds
    /// the union of the four under it.
    #[test]
    fn a_layers_presence_covers_its_block_and_the_blocks_round_it() {
        let (w, h) = (32u32, 32u32);
        let mut one = vec![0u8; (w * h) as usize];
        one[(9 * w + 9) as usize] = 255; // in block (1, 1)
        let none = vec![0u8; (w * h) as usize];
        let (bw, bh, levels) = layer_presence(&[&one, &none], w, h);
        assert_eq!((bw, bh), (4, 4));
        let bit = |x: u32, y: u32| levels[0][(y * bw + x) as usize];
        // Layer 1 in block (1, 1) and the blocks round it, not two blocks away.
        assert_eq!(bit(0, 0), 0b011);
        assert_eq!(bit(2, 2), 0b011);
        assert_eq!(bit(3, 3), 0b001);
        // Layer 2 nowhere; the base everywhere.
        assert!(levels[0].iter().all(|&b| b & 0b100 == 0 && b & 1 == 1));
        // The coarser levels: the union of the four under them, down to one texel.
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[1][3], 0b011, "the far corner's 2 x 2 holds (2, 2)");
        assert_eq!(levels[2], vec![0b011]);
        // An odd size halves rounding down, as Vulkan's levels, the last block in the last texel:
        // 40 x 40 texels are 5 x 5 blocks, then 2 x 2, then 1 x 1.
        let mut corner = vec![0u8; 40 * 40];
        corner[39 * 40 + 39] = 255; // block (4, 4) only
        let (bw, _, levels) = layer_presence(&[&corner], 40, 40);
        assert_eq!(bw, 5);
        assert_eq!(levels.iter().map(Vec::len).collect::<Vec<_>>(), [25, 4, 1]);
        assert_eq!(
            levels[1][3], 0b11,
            "the last texel keeps the odd last block"
        );
        assert_eq!(levels[1][0], 0b01);
    }
}

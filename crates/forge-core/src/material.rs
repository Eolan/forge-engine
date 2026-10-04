//! The material record (D-007): one row per material that every system reads, so that ice
//! looks like ice, slips like ice and sounds like ice from the same entry.
//!
//! A row has a render layer (the shading class, colours, surface, textures), a physics layer
//! (density, friction, restitution) and gameplay tags. The renderer uploads the render layers
//! as a table the visibility resolve indexes by each instance's material id
//! (`forge_render::material`); physics and audio will read the same rows when they land.
//! Weather overrides and the deformable layer join the row with the systems that use them.

use std::fmt;

/// Index of a row in a [`MaterialTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId(pub u32);

/// Index of a texture in the renderer's texture set (`forge_render::material::TextureSet`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureId(pub u32);

/// How a surface is shaded: each class is its own shading pass over the pixels that show it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadingClass {
    /// Opaque, lit by the sun with a soft highlight; optional textures.
    #[default]
    Standard,
    /// Ice: colour from how the surface faces its object's centre, a sharp highlight and
    /// light scattered back where the surface turns away from the sun.
    Ice,
    /// Ground in layers (issue #42): the row's albedo texture is a layer map, one layer id per
    /// texel (`R8_UINT`, `TextureSet::add_layer_map`), laid over `texture_scale` metres of
    /// the object's x and z around its origin. Layer `k` is shaded as the standard row `k + 1`
    /// after this one, and neighbouring layers blend across a texel.
    Layered,
}

impl ShadingClass {
    /// Every class, in the order of their indices.
    pub const ALL: [ShadingClass; 3] = [
        ShadingClass::Standard,
        ShadingClass::Ice,
        ShadingClass::Layered,
    ];

    /// The index the GPU tables use (`MATERIAL_CLASS_*` in `meshlet.slang`).
    pub fn index(self) -> u32 {
        match self {
            ShadingClass::Standard => 0,
            ShadingClass::Ice => 1,
            ShadingClass::Layered => 2,
        }
    }

    /// Lower-case name, as the profiler shows it (`shading/<name>`).
    pub fn name(self) -> &'static str {
        match self {
            ShadingClass::Standard => "standard",
            ShadingClass::Ice => "ice",
            ShadingClass::Layered => "layered",
        }
    }
}

/// How a texture repeats past 0–1 along one axis (glTF's wrapping modes, D-047).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wrap {
    /// Tiles.
    #[default]
    Repeat,
    /// Holds the edge texel.
    Clamp,
    /// Tiles, every other repeat mirrored.
    Mirror,
}

impl Wrap {
    /// Its index in the GPU's table of wrapping samplers (`SAMPLER_ANISOTROPIC_WRAP` in
    /// `bindless.slang`: the sampler for `(u, v)` is `3 u + v` after it).
    pub fn index(self) -> u32 {
        match self {
            Wrap::Repeat => 0,
            Wrap::Clamp => 1,
            Wrap::Mirror => 2,
        }
    }
}

/// A standard row whose textures are read by the mesh's texture coordinates (D-047) instead of
/// projected along the object's axes: a model's material as glTF's metallic-roughness model
/// gives it. The row's `albedo_texture` is the base colour map (times `color_a`), its
/// `normal_texture` the tangent-space normal map (its x and y times `normal_strength`), its
/// `roughness` the roughness factor; a mesh without UVs draws the row's factors alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvMapping {
    /// The UV transform (`KHR_texture_transform`): `u' = t[0] u + t[1] v + t[2]`,
    /// `v' = t[3] u + t[4] v + t[5]`.
    pub transform: [f32; 6],
    /// How the textures repeat along u and v.
    pub wrap: [Wrap; 2],
    /// Metalness, 0 for a dielectric to 1 for a metal. A metal keeps no diffuse colour,
    /// reflects as much as its base colour's brightest channel at normal incidence, and its
    /// highlight weighs [`METAL_SPECULAR`] more than the row's `specular`.
    pub metallic: f32,
    /// Roughness in its green channel and metalness in its blue, multiplying the row's
    /// `roughness` and `metallic`.
    pub metallic_roughness_texture: Option<TextureId>,
    /// Ambient occlusion in its red channel: it darkens the sky's and the probes' light, not
    /// the sun's.
    pub occlusion_texture: Option<TextureId>,
    /// How much of the occlusion applies: `1 + strength × (occlusion − 1)`.
    pub occlusion_strength: f32,
    /// The emitted colour, multiplying the row's `emissive`.
    pub emissive_texture: Option<TextureId>,
}

/// How much more a metal's highlight weighs than a dielectric's ([`UvMapping::metallic`]).
pub const METAL_SPECULAR: f32 = 0.35;

impl Default for UvMapping {
    fn default() -> Self {
        Self {
            transform: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            wrap: [Wrap::Repeat; 2],
            metallic: 0.0,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
        }
    }
}

/// What the renderer needs of a material. Colours are linear, in units of the light a white
/// Lambertian surface facing the sun returns (the resolve scales them to luminance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderLayer {
    /// The shading pass that draws it.
    pub class: ShadingClass,
    /// Base colours. Standard: each instance takes its own mix of the two, from a hash of its
    /// id. Ice: the surface goes from `color_a` in its hollows to `color_b` on its crests.
    pub color_a: [f32; 3],
    /// See `color_a`.
    pub color_b: [f32; 3],
    /// Standard: how much darker the surface gets where it faces its object's centre (0 for
    /// none; meant for rounded objects such as rocks). Layered: how far its layer map's lookup
    /// wanders, in texels, over a noise three texels wide, so the layers' edges do not follow the
    /// map's grid in steps (0: they do, as streets should).
    pub cavity: f32,
    /// Perceptual roughness in [0, 1]: the size of the highlight.
    pub roughness: f32,
    /// Weight of the highlight.
    pub specular: f32,
    /// Light the surface emits, in the units of the colours.
    pub emissive: [f32; 3],
    /// Ice: light scattered back where the surface turns away from the sun.
    pub scatter: [f32; 3],
    /// Standard: an albedo texture multiplied into the base colour, projected along the
    /// object's three axes (no texture coordinates are needed).
    pub albedo_texture: Option<TextureId>,
    /// Standard: a tangent-space normal map projected the same way.
    pub normal_texture: Option<TextureId>,
    /// Metres of object space one repeat of the textures covers.
    pub texture_scale: f32,
    /// How strongly the normal map bends the normal (1 as authored).
    pub normal_strength: f32,
    /// Standard: hide the textures' repeats (issue #66). Hex-tiling (Mikkelsen 2022) samples
    /// them in overlapping hexagonal tiles, each at a random offset and rotation, blended so
    /// that the contrast holds; the projection is also offset per instance, so objects of one
    /// mesh stop sharing a pattern. For stochastic textures (rock, sand, concrete), never for
    /// structured ones (brick courses, tiles, windows), whose rows it would break.
    pub hex_tiling: bool,
    /// The share of light reflected at normal incidence (Fresnel's F0), rising towards 1 at
    /// grazing angles: 0.04 for most dielectrics (stone, plaster, plastic, uncoated glass),
    /// 0.2–0.4 for coated glass (issue #56).
    pub reflectance: f32,
    /// Ice: the share of its volume held in air bubbles and cracks, which scatter the light
    /// inside it (issue #61). 0 is clear ice, which shows the sun through metres of it in
    /// deep blue; about 0.001 is white ice at the scale of a rock, whose surface turns white
    /// and which lets the light through only at its thin edges. Bubbly ice is lighter too:
    /// its density is `917 (1 − bubbles)` kg/m³.
    pub bubbles: f32,
    /// Layered: a layer whose edge follows a height of the drawn ground instead of the map's
    /// texels (the island's sand, #106).
    pub contour: Option<LayerContour>,
    /// Standard: the textures read by the mesh's texture coordinates, with a model's
    /// metalness, occlusion and emission (D-047); `None` projects them.
    pub uv: Option<UvMapping>,
}

/// A layer of a [`ShadingClass::Layered`] row drawn by the ground's height under each pixel: the
/// rule that made the layer map, at the drawn mesh's resolution. Where the map shows `below`, one
/// of `others` or one of `above`, the pixel takes a layer under `height` (the map's own when it
/// shows `below` or one of `others`, the heaviest of them around it otherwise, `below` where it
/// shows none) and an `above` layer over it (the map's own when it shows one, the heaviest around
/// it otherwise); the other layers keep the map's edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerContour {
    /// The layer under the height where the map shows no other one of them.
    pub below: u8,
    /// The other layers under the height, one bit each (bit `k` for layer `k`): the island's
    /// beaches but its sand (#128).
    pub others: u32,
    /// The layers over the height, one bit each (bit `k` for layer `k`).
    pub above: u32,
    /// Metres of object space.
    pub height: f32,
    /// How far the height wanders over a noise a few metres wide, metres, so the edge does not
    /// run as clean as a contour line.
    pub wander: f32,
}

impl LayerContour {
    /// `below` under `height` and the layers of `above` over it (layers 0 to 31), the height
    /// wandering by `wander` metres.
    pub fn new(below: u8, above: &[u8], height: f32, wander: f32) -> Self {
        assert!(
            above.iter().all(|&layer| layer < 32),
            "layers 0 to 31 over a contour"
        );
        Self {
            below,
            others: 0,
            above: above.iter().fold(0, |mask, &layer| mask | 1 << layer),
            height,
            wander,
        }
    }

    /// The same contour with `others` under the height besides `below` (layers 0 to 31).
    pub fn with_others(mut self, others: &[u8]) -> Self {
        assert!(
            others.iter().all(|&layer| layer < 32),
            "layers 0 to 31 under a contour"
        );
        self.others = others.iter().fold(0, |mask, &layer| mask | 1 << layer);
        self
    }
}

impl Default for RenderLayer {
    fn default() -> Self {
        Self {
            class: ShadingClass::Standard,
            color_a: [0.5; 3],
            color_b: [0.5; 3],
            cavity: 0.0,
            roughness: 0.6,
            specular: 0.05,
            emissive: [0.0; 3],
            scatter: [0.0; 3],
            albedo_texture: None,
            normal_texture: None,
            texture_scale: 1.0,
            normal_strength: 1.0,
            hex_tiling: false,
            reflectance: 0.04,
            bubbles: 0.0,
            contour: None,
            uv: None,
        }
    }
}

impl RenderLayer {
    /// The Blinn-Phong exponent of `roughness` (α = roughness², exponent = 2 / α² − 2),
    /// rounded to a thousandth so that rows written from an exponent give it back exactly. At least a
    /// thousandth: at roughness 1 the exponent is 0, and the shaders' `pow(0, 0)` is NaN.
    pub fn specular_power(&self) -> f32 {
        let alpha = f64::from(self.roughness.clamp(0.02, 1.0)).powi(2);
        let power = 2.0 / (alpha * alpha) - 2.0;
        ((power * 1000.0).round() / 1000.0).max(0.001) as f32
    }

    /// The roughness whose [`RenderLayer::specular_power`] is `power`.
    pub fn roughness_for_power(power: f32) -> f32 {
        (2.0 / (f64::from(power) + 2.0)).sqrt().sqrt() as f32
    }
}

/// What physics will read of a material (`forge-physics`, Phase 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsLayer {
    /// kg/m³.
    pub density: f32,
    /// Friction coefficient at rest.
    pub static_friction: f32,
    /// Friction coefficient while sliding.
    pub dynamic_friction: f32,
    /// Share of the normal speed kept by a bounce (0 to 1).
    pub restitution: f32,
}

impl Default for PhysicsLayer {
    fn default() -> Self {
        // Dry rock.
        Self {
            density: 2600.0,
            static_friction: 0.7,
            dynamic_friction: 0.6,
            restitution: 0.2,
        }
    }
}

/// Gameplay tags of a material, as bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MaterialTags(pub u32);

impl MaterialTags {
    /// Low friction for characters and wheels.
    pub const SLIPPERY: MaterialTags = MaterialTags(1);
    /// Takes footprints and tracks (snow, sand, mud).
    pub const DEFORMABLE: MaterialTags = MaterialTags(2);
    /// Burns.
    pub const FLAMMABLE: MaterialTags = MaterialTags(4);
    /// Can be climbed.
    pub const CLIMBABLE: MaterialTags = MaterialTags(8);
    /// Floats.
    pub const BUOYANT: MaterialTags = MaterialTags(16);
    /// Characters sink into it.
    pub const SINKABLE: MaterialTags = MaterialTags(32);

    /// Whether every bit of `other` is set.
    pub fn contains(self, other: MaterialTags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for MaterialTags {
    type Output = MaterialTags;
    fn bitor(self, other: MaterialTags) -> MaterialTags {
        MaterialTags(self.0 | other.0)
    }
}

/// One material: every system's view of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    /// A name for tools and logs.
    pub name: String,
    /// How it is drawn.
    pub render: RenderLayer,
    /// How it moves and collides.
    pub physics: PhysicsLayer,
    /// Gameplay tags.
    pub tags: MaterialTags,
}

impl Material {
    /// A material named `name` drawn as `render`, with default physics and no tags.
    pub fn new(name: &str, render: RenderLayer) -> Self {
        Self {
            name: name.to_owned(),
            render,
            physics: PhysicsLayer::default(),
            tags: MaterialTags::default(),
        }
    }
}

/// The rows of a world's materials. Row 0 always exists: a plain grey default for anything
/// that names none.
#[derive(Clone, Debug)]
pub struct MaterialTable {
    rows: Vec<Material>,
}

impl Default for MaterialTable {
    fn default() -> Self {
        Self {
            rows: vec![Material::new("default", RenderLayer::default())],
        }
    }
}

impl MaterialTable {
    /// The default row.
    pub const DEFAULT: MaterialId = MaterialId(0);

    /// A table with only the default row.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a row.
    pub fn add(&mut self, material: Material) -> MaterialId {
        self.rows.push(material);
        MaterialId(self.rows.len() as u32 - 1)
    }

    /// The row `id`.
    pub fn get(&self, id: MaterialId) -> &Material {
        &self.rows[id.0 as usize]
    }

    /// The row named `name`, if any.
    pub fn find(&self, name: &str) -> Option<MaterialId> {
        self.rows
            .iter()
            .position(|m| m.name == name)
            .map(|i| MaterialId(i as u32))
    }

    /// Every row, in id order.
    pub fn rows(&self) -> &[Material] {
        &self.rows
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Always false: the default row is there.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl fmt::Display for MaterialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "material {}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exponent_survives_the_trip_through_roughness() {
        for power in [4.0_f32, 14.0, 48.0, 200.0] {
            let layer = RenderLayer {
                roughness: RenderLayer::roughness_for_power(power),
                ..RenderLayer::default()
            };
            assert_eq!(layer.specular_power(), power);
        }
    }

    #[test]
    fn rougher_surfaces_have_wider_highlights() {
        let power = |roughness| {
            RenderLayer {
                roughness,
                ..RenderLayer::default()
            }
            .specular_power()
        };
        assert!(power(0.2) > power(0.5));
        assert!(power(0.5) > power(0.9));
        assert!(power(1.0) >= 0.0);
    }

    #[test]
    fn a_contour_names_its_layers_over_the_height_one_bit_each() {
        let contour = LayerContour::new(1, &[0, 5, 6], 2.5, 0.3);
        assert_eq!(contour.below, 1);
        assert_eq!(contour.above, 0b110_0001);
        assert_eq!(LayerContour::new(1, &[], 2.5, 0.0).above, 0);
        // The other layers under the height, the island's other beaches (#128).
        assert_eq!(contour.others, 0);
        let beaches = contour.with_others(&[13, 14]);
        assert_eq!(beaches.others, 0b110 << 12);
        assert_eq!((beaches.below, beaches.above), (1, 0b110_0001));
    }

    #[test]
    fn the_table_starts_with_its_default_row() {
        let mut table = MaterialTable::new();
        assert_eq!(table.len(), 1);
        assert_eq!(table.get(MaterialTable::DEFAULT).name, "default");
        let ice = table.add(Material::new(
            "ice",
            RenderLayer {
                class: ShadingClass::Ice,
                ..RenderLayer::default()
            },
        ));
        assert_eq!(ice, MaterialId(1));
        assert_eq!(table.find("ice"), Some(ice));
        assert_eq!(table.get(ice).render.class.index(), 1);
    }

    #[test]
    fn the_standard_class_comes_first_and_indices_follow_the_order() {
        // The renderer shades the first class over the whole target and the others in tiles.
        assert_eq!(ShadingClass::ALL[0], ShadingClass::Standard);
        for (i, class) in ShadingClass::ALL.iter().enumerate() {
            assert_eq!(class.index() as usize, i);
        }
    }

    #[test]
    fn tags_combine() {
        let tags = MaterialTags::SLIPPERY | MaterialTags::BUOYANT;
        assert!(tags.contains(MaterialTags::SLIPPERY));
        assert!(!tags.contains(MaterialTags::FLAMMABLE));
    }
}

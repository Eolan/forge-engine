//! glTF 2.0 models into Forge's meshes (issue #138): the binary form (`.glb`) that Blender and
//! most tools export, read through the `gltf` crate. Each mesh of the scene comes out as a
//! [`TriMesh`] in the scene's frame (its nodes' transforms applied), its primitives as material
//! sections, with the materials' base colour, roughness and metalness to make rows of, and
//! (D-047) the texture coordinates, the embedded images and the textures each material
//! samples, with their samplers and transforms as the artist set them. A skinned mesh keeps
//! its vertices' joints (#165), and a mesh its morph targets, their names and weights (#169).
//!
//! glTF is +Y up, metres, and a model's front faces `+Z`; Blender's exporter turns its +Y
//! (forward) into glTF's `−Z`, which is Forge's forward too.

use std::path::{Path, PathBuf};

use glam::{Mat3, Mat4, Vec3};

use crate::procedural::TriMesh;
use crate::skin::{MorphTarget, VertexSkin};

/// Why a model did not load.
#[derive(Debug, thiserror::Error)]
pub enum GltfError {
    /// The bytes are not a glTF file the reader understands.
    #[error("not a glTF model: {0}")]
    Format(#[from] gltf::Error),
    /// The file refers to data outside itself (Forge reads `.glb` with its buffer inside).
    #[error("the model's buffer {0} is not in the file")]
    ExternalBuffer(usize),
    /// A primitive has no positions, or is not made of triangles.
    #[error("mesh {mesh}: a primitive without triangles")]
    NotTriangles {
        /// The mesh's name.
        mesh: String,
    },
    /// More materials in one mesh than a section number holds.
    #[error("mesh {mesh}: more than 256 materials")]
    TooManyMaterials {
        /// The mesh's name.
        mesh: String,
    },
    /// The file refers to an image outside itself (and was read from memory, or by a `data:`
    /// URI, which Forge does not read).
    #[error("the model's image {0} is not in the file")]
    ExternalImage(usize),
    /// A file the model names (or the model) could not be read.
    #[error("{path}: {source}", path = .0.display(), source = .1)]
    Io(PathBuf, #[source] std::io::Error),
    /// A skinned primitive's joints or weights are not one per vertex.
    #[error("mesh {mesh}: not one joint set and one weight set per vertex")]
    BadSkin {
        /// The mesh's name.
        mesh: String,
    },
    /// A morph target's changes are not one per vertex of its primitive.
    #[error("mesh {mesh}: a morph target without one change per vertex")]
    BadMorph {
        /// The mesh's name.
        mesh: String,
    },
}

/// A material as the model gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    /// Its name ("" when unnamed).
    pub name: String,
    /// Linear RGBA.
    pub base_color: [f32; 4],
    /// Perceptual roughness, 0 to 1.
    pub roughness: f32,
    /// 0 for a dielectric, 1 for a metal.
    pub metallic: f32,
    /// The light it emits, linear RGB: the emissive factor times its strength
    /// (`KHR_materials_emissive_strength`, Blender's emission strength). Forge reads it in units
    /// of the light a white surface facing the sun returns.
    pub emissive: [f32; 3],
    /// The base colour texture (sRGB), multiplied by `base_color`.
    pub base_color_texture: Option<TextureRef>,
    /// Roughness in its green channel and metalness in its blue (linear), multiplied by
    /// `roughness` and `metallic`.
    pub metallic_roughness_texture: Option<TextureRef>,
    /// A tangent-space normal map (linear).
    pub normal_texture: Option<TextureRef>,
    /// How far the normal map bends the normal (its x and y are scaled by this).
    pub normal_scale: f32,
    /// Ambient occlusion in its red channel (linear).
    pub occlusion_texture: Option<TextureRef>,
    /// How much of the occlusion applies: `1 + strength × (occlusion − 1)`.
    pub occlusion_strength: f32,
    /// The emitted colour (sRGB), multiplied by `emissive`.
    pub emissive_texture: Option<TextureRef>,
    /// Where the base colour's alpha (its factor times its texture's) cuts the surface out:
    /// glTF's `MASK` with its `alphaCutoff`; `None` for an opaque one (and a blended one, which
    /// Forge draws opaque, #171).
    pub alpha_cutoff: Option<f32>,
    /// Seen from both sides (glTF's `doubleSided`): not culled from behind, its normal turned
    /// towards the viewer.
    pub double_sided: bool,
}

pub use forge_core::material::Wrap;

/// A material's use of one of the model's images.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureRef {
    /// Its image in [`Model::images`].
    pub image: usize,
    /// How it repeats along u and v.
    pub wrap: [Wrap; 2],
    /// Its UV transform (`KHR_texture_transform`): `u' = t[0] u + t[1] v + t[2]`,
    /// `v' = t[3] u + t[4] v + t[5]`; [`UV_IDENTITY`] when the file gives none.
    pub transform: [f32; 6],
}

/// The identity UV transform of a [`TextureRef`].
pub const UV_IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// An image embedded in the model, still encoded.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelImage {
    /// Its name, or "" when unnamed.
    pub name: String,
    /// Its MIME type (`image/png`, `image/jpeg`).
    pub mime: String,
    /// The encoded bytes.
    pub bytes: Vec<u8>,
}

/// One mesh of a model, in the scene's frame.
#[derive(Clone, Debug)]
pub struct ModelMesh {
    /// The name of the node that holds it, or the mesh's.
    pub name: String,
    /// Its triangles; section `s` is drawn with `materials[s]`.
    pub mesh: TriMesh,
    /// Its materials, one per section.
    pub materials: Vec<ModelMaterial>,
    /// For a skinned mesh (#165), each vertex's joints in its node's skin. Its vertices
    /// are then in the bind pose as the file gives them: glTF moves a skinned mesh by its
    /// joints alone, not by its node.
    pub skin: Option<Vec<VertexSkin>>,
    /// For a skinned mesh whose vertices follow up to eight joints (#169), each vertex's second
    /// four (glTF's `JOINTS_1` and `WEIGHTS_1`; zero weights where a primitive has none): the
    /// eight weights together sum to 1. [`SkinnedMesh::cook_full`] takes them.
    ///
    /// [`SkinnedMesh::cook_full`]: crate::SkinnedMesh::cook_full
    pub skin_more: Option<Vec<VertexSkin>>,
    /// Its morph targets (#169), each with a change per vertex, turned into the scene's frame
    /// as the vertices are; empty for none. [`SkinnedMesh::cook_morphed`] takes them.
    ///
    /// [`SkinnedMesh::cook_morphed`]: crate::SkinnedMesh::cook_morphed
    pub morphs: Vec<MorphTarget>,
    /// The file's weights of its targets, which `mesh` does not include: its vertices are the
    /// targets' base.
    pub morph_weights: Vec<f32>,
}

/// The meshes of a model.
#[derive(Clone, Debug, Default)]
pub struct Model {
    /// In the order the scene's nodes list them.
    pub meshes: Vec<ModelMesh>,
    /// The named nodes that hold no mesh (Blender's empties: a joint's pivot, a socket), with
    /// where they are in the scene's frame (#143).
    pub points: Vec<(String, Vec3)>,
    /// The images its materials sample.
    pub images: Vec<ModelImage>,
    /// What the file asks for that Forge does not draw yet (blended alpha, a second UV set),
    /// one line each, for the caller to log.
    pub unsupported: Vec<String>,
}

impl Model {
    /// The mesh named `name`.
    pub fn mesh(&self, name: &str) -> Option<&ModelMesh> {
        self.meshes.iter().find(|m| m.name == name)
    }

    /// The point (an empty) named `name`.
    pub fn point(&self, name: &str) -> Option<Vec3> {
        self.points.iter().find(|(n, _)| n == name).map(|&(_, p)| p)
    }
}

/// Reads a binary glTF (`.glb`): every mesh the default scene's nodes hold, with their nodes'
/// transforms applied. Its buffers and images must be inside it.
pub fn load_glb(bytes: &[u8]) -> Result<Model, GltfError> {
    load(bytes, None)
}

/// Reads a glTF file, binary (`.glb`) or JSON (`.gltf`), with the buffers and images it names
/// beside it (D-048: the Khronos sample models' `.gltf` folders).
pub fn load_gltf(path: &Path) -> Result<Model, GltfError> {
    let bytes = std::fs::read(path).map_err(|e| GltfError::Io(path.to_owned(), e))?;
    load(&bytes, Some(path.parent().unwrap_or(Path::new("."))))
}

/// The file at `uri` beside the model in `dir` (`%XX` escapes decoded); a `data:` URI or a
/// model read from memory has none.
fn beside(dir: Option<&Path>, uri: &str) -> Option<PathBuf> {
    if uri.starts_with("data:") {
        return None;
    }
    let bytes = uri.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                decoded.push(b);
                i += 3;
            }
            (b, _) => {
                decoded.push(b);
                i += 1;
            }
        }
    }
    Some(dir?.join(String::from_utf8_lossy(&decoded).as_ref()))
}

fn load(bytes: &[u8], dir: Option<&Path>) -> Result<Model, GltfError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let mut buffers = Vec::new();
    for buffer in gltf.buffers() {
        buffers.push(match buffer.source() {
            gltf::buffer::Source::Bin => gltf
                .blob
                .clone()
                .ok_or(GltfError::ExternalBuffer(buffer.index()))?,
            gltf::buffer::Source::Uri(uri) => {
                let path = beside(dir, uri).ok_or(GltfError::ExternalBuffer(buffer.index()))?;
                std::fs::read(&path).map_err(|e| GltfError::Io(path, e))?
            }
        });
    }
    let mut model = Model::default();
    for image in gltf.images() {
        model.images.push(read_image(&image, &buffers, dir)?);
    }
    let buffers = buffers.as_slice();
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .into_iter();
    for scene in scene {
        for node in scene.nodes() {
            visit(&node, Mat4::IDENTITY, buffers, &mut model)?;
        }
    }
    model.unsupported.sort();
    model.unsupported.dedup();
    Ok(model)
}

fn visit(
    node: &gltf::Node,
    parent: Mat4,
    buffers: &[Vec<u8>],
    model: &mut Model,
) -> Result<(), GltfError> {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        let name = node.name().or(mesh.name()).unwrap_or_default().to_owned();
        // A skinned mesh stays in its bind pose: its joints place it.
        let skinned = node.skin().is_some();
        let frame = if skinned { Mat4::IDENTITY } else { world };
        model.meshes.push(read_mesh(
            &mesh,
            frame,
            name,
            skinned,
            buffers,
            &mut model.unsupported,
        )?);
    } else if let Some(name) = node.name() {
        model
            .points
            .push((name.to_owned(), world.transform_point3(Vec3::ZERO)));
    }
    for child in node.children() {
        visit(&child, world, buffers, model)?;
    }
    Ok(())
}

fn read_mesh(
    mesh: &gltf::Mesh,
    world: Mat4,
    name: String,
    skinned: bool,
    buffers: &[Vec<u8>],
    unsupported: &mut Vec<String>,
) -> Result<ModelMesh, GltfError> {
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    // Mirrored transforms turn the triangles inside out: wind them back.
    let flip = world.determinant() < 0.0;
    let mut out = TriMesh::default();
    let mut materials: Vec<ModelMaterial> = Vec::new();
    let mut skin: Vec<VertexSkin> = Vec::new();
    let mut more: Vec<VertexSkin> = Vec::new();
    let mut any_more = false;
    let mut morphs: Vec<MorphTarget> = Vec::new();
    let mut needs_normals = false;
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut any_uvs = false;
    for primitive in mesh.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            return Err(GltfError::NotTriangles { mesh: name });
        }
        let reader = primitive.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
        let positions: Vec<Vec3> = reader
            .read_positions()
            .ok_or_else(|| GltfError::NotTriangles { mesh: name.clone() })?
            .map(|p| world.transform_point3(Vec3::from(p)))
            .collect();
        let normals: Option<Vec<Vec3>> = reader.read_normals().map(|n| {
            n.map(|n| (normal_matrix * Vec3::from(n)).normalize_or(Vec3::Y))
                .collect()
        });
        let indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..positions.len() as u32).collect(),
        };
        let material = primitive.material();
        let this = read_material(&material, &name, unsupported);
        let section = match materials.iter().position(|m| *m == this) {
            Some(s) => s,
            None => {
                materials.push(this);
                materials.len() - 1
            }
        };
        let section = u8::try_from(section)
            .map_err(|_| GltfError::TooManyMaterials { mesh: name.clone() })?;
        if skinned {
            // A primitive without joints follows the skin's first joint.
            let joints: Vec<[u16; 4]> = match reader.read_joints(0) {
                Some(j) => j.into_u16().collect(),
                None => vec![[0; 4]; positions.len()],
            };
            let weights: Vec<[f32; 4]> = match reader.read_weights(0) {
                Some(w) => w.into_f32().collect(),
                None => vec![[1.0, 0.0, 0.0, 0.0]; positions.len()],
            };
            if joints.len() != positions.len() || weights.len() != positions.len() {
                return Err(GltfError::BadSkin { mesh: name });
            }
            skin.extend(
                joints
                    .into_iter()
                    .zip(weights)
                    .map(|(joints, weights)| VertexSkin { joints, weights }),
            );
            // A second set (#169): its vertices follow up to eight joints. The primitives
            // before without one follow none more.
            if let (Some(j), Some(w)) = (reader.read_joints(1), reader.read_weights(1)) {
                let joints: Vec<[u16; 4]> = j.into_u16().collect();
                let weights: Vec<[f32; 4]> = w.into_f32().collect();
                if joints.len() != positions.len() || weights.len() != positions.len() {
                    return Err(GltfError::BadSkin { mesh: name });
                }
                more.resize(skin.len() - positions.len(), VertexSkin::default());
                more.extend(
                    joints
                        .into_iter()
                        .zip(weights)
                        .map(|(joints, weights)| VertexSkin { joints, weights }),
                );
                any_more = true;
            }
        }
        match reader.read_tex_coords(0) {
            Some(t) => {
                any_uvs = true;
                uvs.extend(t.into_f32());
            }
            None => uvs.extend(std::iter::repeat_n([0.0, 0.0], positions.len())),
        }
        if reader.read_tex_coords(1).is_some() {
            unsupported.push(format!(
                "mesh {name}: a second UV set (TEXCOORD_1) is ignored"
            ));
        }
        // Its morph targets' changes, turned as its vertices and normals are, after the
        // primitives before's (whose changes a target this one adds are zero).
        let linear = Mat3::from_mat4(world);
        let before = out.positions.len();
        let after = before + positions.len();
        let mut given = 0;
        for (t, (moved, turned, _)) in reader.read_morph_targets().enumerate() {
            given = t + 1;
            if t == morphs.len() {
                morphs.push(MorphTarget {
                    name: String::new(),
                    positions: vec![[0.0; 3]; before],
                    normals: vec![[0.0; 3]; before],
                });
            }
            let target = &mut morphs[t];
            match moved {
                Some(m) => target
                    .positions
                    .extend(m.map(|d| (linear * Vec3::from(d)).to_array())),
                None => target
                    .positions
                    .extend(std::iter::repeat_n([0.0; 3], positions.len())),
            }
            match turned {
                Some(n) => target
                    .normals
                    .extend(n.map(|d| (normal_matrix * Vec3::from(d)).to_array())),
                None => target
                    .normals
                    .extend(std::iter::repeat_n([0.0; 3], positions.len())),
            }
            if target.positions.len() != after || target.normals.len() != after {
                return Err(GltfError::BadMorph { mesh: name });
            }
        }
        // A target this primitive lacks moves none of its vertices.
        for target in &mut morphs[given..] {
            target.positions.resize(after, [0.0; 3]);
            target.normals.resize(after, [0.0; 3]);
        }
        let first = out.positions.len() as u32;
        out.positions.extend(positions.iter().map(|p| p.to_array()));
        match &normals {
            Some(n) => out.normals.extend(n.iter().map(|n| n.to_array())),
            None => {
                needs_normals = true;
                out.normals
                    .extend(std::iter::repeat_n([0.0, 1.0, 0.0], positions.len()));
            }
        }
        for t in indices.as_chunks::<3>().0 {
            let (a, b, c) = (first + t[0], first + t[1], first + t[2]);
            if flip {
                out.indices.extend_from_slice(&[a, c, b]);
            } else {
                out.indices.extend_from_slice(&[a, b, c]);
            }
            out.sections.push(section);
        }
    }
    if needs_normals {
        out.recompute_normals();
    }
    if any_uvs {
        if uvs.len() != out.positions.len() {
            return Err(GltfError::NotTriangles { mesh: name });
        }
        out.uvs = uvs;
    }
    // One material: no sections needed.
    if materials.len() <= 1 {
        out.sections.clear();
    }
    // The targets' names, which Blender and most exporters put in the mesh's extras.
    let names: Vec<String> = mesh
        .extras()
        .as_ref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok())
        .and_then(|extras| serde_json::from_value(extras.get("targetNames")?.clone()).ok())
        .unwrap_or_default();
    for (target, name) in morphs.iter_mut().zip(names) {
        target.name = name;
    }
    for target in &mut morphs {
        if target.normals.iter().all(|n| *n == [0.0; 3]) {
            target.normals.clear();
        }
    }
    // The primitives after the last with a second set follow none more.
    more.resize(skin.len(), VertexSkin::default());
    let mut morph_weights = mesh.weights().map(<[f32]>::to_vec).unwrap_or_default();
    morph_weights.resize(morphs.len(), 0.0);
    Ok(ModelMesh {
        name,
        mesh: out,
        materials,
        skin: skinned.then_some(skin),
        skin_more: any_more.then_some(more),
        morphs,
        morph_weights,
    })
}

/// What a texture slot of a material refers to: its texture, UV set and transform.
type Slot<'a> = (gltf::texture::Texture<'a>, u32, Option<[f32; 6]>);

/// The material `material` of mesh `mesh` as Forge reads it; what it cannot draw yet goes to
/// `unsupported`.
fn read_material(
    material: &gltf::Material,
    mesh: &str,
    unsupported: &mut Vec<String>,
) -> ModelMaterial {
    let pbr = material.pbr_metallic_roughness();
    let strength = material.emissive_strength().unwrap_or(1.0);
    let label = material.name().unwrap_or("unnamed").to_owned();
    if material.alpha_mode() == gltf::material::AlphaMode::Blend {
        unsupported.push(format!(
            "material {label} of mesh {mesh}: alpha Blend is drawn opaque"
        ));
    }
    let alpha_cutoff = (material.alpha_mode() == gltf::material::AlphaMode::Mask)
        .then(|| material.alpha_cutoff().unwrap_or(0.5));
    let mut texture = |slot: Option<Slot>, what: &str| {
        let (texture, tex_coord, transform) = slot?;
        if tex_coord != 0 {
            unsupported.push(format!(
                "material {label} of mesh {mesh}: its {what} texture reads UV set {tex_coord}, not drawn"
            ));
            return None;
        }
        let sampler = texture.sampler();
        if sampler.mag_filter() == Some(gltf::texture::MagFilter::Nearest) {
            unsupported.push(format!(
                "material {label} of mesh {mesh}: its {what} texture's nearest filtering is drawn linear"
            ));
        }
        let wrap = |w: gltf::texture::WrappingMode| match w {
            gltf::texture::WrappingMode::Repeat => Wrap::Repeat,
            gltf::texture::WrappingMode::ClampToEdge => Wrap::Clamp,
            gltf::texture::WrappingMode::MirroredRepeat => Wrap::Mirror,
        };
        Some(TextureRef {
            image: texture.source().index(),
            wrap: [wrap(sampler.wrap_s()), wrap(sampler.wrap_t())],
            transform: transform.unwrap_or(UV_IDENTITY),
        })
    };
    // KHR_texture_transform: translation × rotation × scale. The rotation turns the UVs from +u
    // towards −v (u' = cos u + sin v), as Khronos's TextureTransformTest checks (#170: the
    // opposite sign pointed its arrows at the red crosses).
    fn info(i: gltf::texture::Info<'_>) -> Slot<'_> {
        let transform = i.texture_transform().map(|t| {
            let (s, c) = t.rotation().sin_cos();
            let [sx, sy] = t.scale();
            let [ox, oy] = t.offset();
            [c * sx, s * sy, ox, -s * sx, c * sy, oy]
        });
        (i.texture(), i.tex_coord(), transform)
    }
    let base_color_texture = texture(pbr.base_color_texture().map(info), "base colour");
    let metallic_roughness_texture = texture(
        pbr.metallic_roughness_texture().map(info),
        "metallic-roughness",
    );
    let emissive_texture = texture(material.emissive_texture().map(info), "emissive");
    // The normal and occlusion textures take the base colour's transform: the `gltf` crate
    // gives theirs only through the raw extensions.
    let shared = base_color_texture.map(|t| t.transform);
    let normal = material.normal_texture();
    let normal_scale = normal.as_ref().map_or(1.0, |n| n.scale());
    let normal_texture = texture(
        normal.map(|n| (n.texture(), n.tex_coord(), shared)),
        "normal",
    );
    let occlusion = material.occlusion_texture();
    let occlusion_strength = occlusion.as_ref().map_or(1.0, |o| o.strength());
    let occlusion_texture = texture(
        occlusion.map(|o| (o.texture(), o.tex_coord(), shared)),
        "occlusion",
    );
    ModelMaterial {
        name: material.name().unwrap_or_default().to_owned(),
        base_color: pbr.base_color_factor(),
        roughness: pbr.roughness_factor(),
        metallic: pbr.metallic_factor(),
        emissive: material.emissive_factor().map(|c| c * strength),
        base_color_texture,
        metallic_roughness_texture,
        normal_texture,
        normal_scale,
        occlusion_texture,
        occlusion_strength,
        emissive_texture,
        alpha_cutoff,
        double_sided: material.double_sided(),
    }
}

/// Image `image`'s encoded bytes, from the file's buffer.
fn read_image(
    image: &gltf::Image,
    buffers: &[Vec<u8>],
    dir: Option<&Path>,
) -> Result<ModelImage, GltfError> {
    let name = image.name().unwrap_or_default().to_owned();
    match image.source() {
        gltf::image::Source::View { view, mime_type } => {
            let missing = GltfError::ExternalBuffer(view.buffer().index());
            let bytes = buffers
                .get(view.buffer().index())
                .and_then(|b| b.get(view.offset()..view.offset() + view.length()))
                .ok_or(missing)?;
            Ok(ModelImage {
                name,
                mime: mime_type.to_owned(),
                bytes: bytes.to_vec(),
            })
        }
        gltf::image::Source::Uri { uri, mime_type } => {
            let path = beside(dir, uri).ok_or(GltfError::ExternalImage(image.index()))?;
            let bytes = std::fs::read(&path).map_err(|e| GltfError::Io(path.clone(), e))?;
            let extension = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            let mime = mime_type.map(str::to_owned).unwrap_or_else(|| {
                match extension.to_ascii_lowercase().as_str() {
                    "jpg" | "jpeg" => "image/jpeg",
                    "png" => "image/png",
                    _ => "",
                }
                .to_owned()
            });
            let name = if name.is_empty() {
                uri.to_owned()
            } else {
                name
            };
            Ok(ModelImage { name, mime, bytes })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-triangle glTF built by hand: a node turned a quarter round +y and moved 2 m up, its
    /// mesh's buffer inside the file.
    fn triangle_glb() -> Vec<u8> {
        let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]];
        let mut bin: Vec<u8> = positions
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"tri","mesh":0,"translation":[0,2,0],"rotation":[0,{s},0,{s}]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"material":0}}]}}],
            "materials":[{{"name":"red","pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1],"roughnessFactor":0.4,"metallicFactor":0}}}}],
            "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,-1],"max":[1,0,0]}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}}],
            "buffers":[{{"byteLength":{}}}]}}"#,
            bin.len()
        );
        pack(json, bin)
    }

    /// `json` and `bin` (padded to 4 bytes) as a `.glb`.
    fn pack(json: String, bin: Vec<u8>) -> Vec<u8> {
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&(total as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    #[test]
    fn a_node_s_transform_and_its_material_come_through() {
        let model = load_glb(&triangle_glb()).unwrap();
        let tri = model.mesh("tri").unwrap();
        assert_eq!(tri.mesh.triangle_count(), 1);
        // A quarter turn round +y takes +x to −z, and the node stands 2 m up.
        let p = Vec3::from(tri.mesh.positions[1]);
        assert!(p.abs_diff_eq(Vec3::new(0.0, 2.0, -1.0), 1e-5), "{p}");
        // No normals in the file: computed, facing up for this winding.
        assert!(Vec3::from(tri.mesh.normals[0]).abs_diff_eq(Vec3::Y, 1e-5));
        assert_eq!(tri.materials.len(), 1);
        assert_eq!(tri.materials[0].name, "red");
        assert_eq!(tri.materials[0].base_color, [1.0, 0.0, 0.0, 1.0]);
        assert!((tri.materials[0].roughness - 0.4).abs() < 1e-6);
    }

    #[test]
    fn a_skinned_mesh_keeps_its_bind_pose_and_its_joints() {
        // One triangle on a node moved 5 m up (which glTF ignores for a skinned mesh), its
        // corners on joints 0, 1 and half of each, the joints as bytes.
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        bin.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0]);
        let weights: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0];
        bin.extend(weights.iter().flat_map(|v| v.to_le_bytes()));
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,1]}}],
            "nodes":[{{"name":"body","mesh":0,"skin":0,"translation":[0,5,0]}},
                     {{"name":"root","children":[2]}},{{"name":"tip","translation":[1,0,0]}}],
            "skins":[{{"joints":[1,2]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}}}}]}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
              {{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"}},
              {{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}},
                           {{"buffer":0,"byteOffset":36,"byteLength":12}},
                           {{"buffer":0,"byteOffset":48,"byteLength":48}}],
            "buffers":[{{"byteLength":{}}}]}}"#,
            bin.len()
        );
        let model = load_glb(&pack(json, bin)).unwrap();
        let body = model.mesh("body").unwrap();
        assert_eq!(body.mesh.positions[1], [1.0, 0.0, 0.0]);
        let skin = body.skin.as_ref().unwrap();
        assert_eq!(skin.len(), 3);
        assert_eq!(skin[1].joints, [1, 0, 0, 0]);
        assert_eq!(skin[2].weights, [0.5, 0.5, 0.0, 0.0]);
        // The joints are points too, and an unskinned mesh has no skin.
        assert!(model.point("tip").is_some());
        let model = load_glb(&triangle_glb()).unwrap();
        assert!(model.mesh("tri").unwrap().skin.is_none());
        assert!(
            body.skin_more.is_none(),
            "four joints a vertex: no second set"
        );
    }

    #[test]
    fn a_vertex_s_second_joint_set_comes_through() {
        // One triangle on four joints: its last corner shares itself among all four, its two
        // sets (JOINTS_0 and JOINTS_1) a quarter each.
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        bin.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0]);
        let weights: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.25, 0.25, 0.0, 0.0];
        bin.extend(weights.iter().flat_map(|v| v.to_le_bytes()));
        bin.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 0, 0]);
        let more: [f32; 12] = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.25, 0.25, 0.0, 0.0];
        bin.extend(more.iter().flat_map(|v| v.to_le_bytes()));
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,1]}}],
            "nodes":[{{"name":"body","mesh":0,"skin":0}},
                     {{"name":"root","children":[2,3,4]}},{{"name":"a"}},{{"name":"b"}},{{"name":"c"}}],
            "skins":[{{"joints":[1,2,3,4]}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2,"JOINTS_1":3,"WEIGHTS_1":4}}}}]}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
              {{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"}},
              {{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}},
              {{"bufferView":3,"componentType":5121,"count":3,"type":"VEC4"}},
              {{"bufferView":4,"componentType":5126,"count":3,"type":"VEC4"}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}},
                           {{"buffer":0,"byteOffset":36,"byteLength":12}},
                           {{"buffer":0,"byteOffset":48,"byteLength":48}},
                           {{"buffer":0,"byteOffset":96,"byteLength":12}},
                           {{"buffer":0,"byteOffset":108,"byteLength":48}}],
            "buffers":[{{"byteLength":{}}}]}}"#,
            bin.len()
        );
        let model = load_glb(&pack(json, bin)).unwrap();
        let body = model.mesh("body").unwrap();
        let skin = body.skin.as_ref().unwrap();
        let more = body.skin_more.as_ref().expect("a second set");
        assert_eq!(more.len(), 3);
        assert_eq!(
            (skin[2].joints, more[2].joints),
            ([0, 1, 0, 0], [2, 3, 0, 0])
        );
        assert_eq!(more[2].weights, [0.25, 0.25, 0.0, 0.0]);
        assert_eq!(more[0].weights, [0.0; 4]);
    }

    #[test]
    fn a_mesh_s_morph_targets_come_through_turned_named_and_weighted() {
        // A triangle drawn twice (two primitives), on a node turned a quarter round +y. The
        // first primitive has two targets: "smile" moves its second corner 10 cm along +x,
        // "blink" lifts every corner 20 cm and tilts its normal towards +z. The second has none.
        let floats: [f32; 36] = [
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, // positions
            0.0, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, // smile
            0.0, 0.2, 0.0, 0.0, 0.2, 0.0, 0.0, 0.2, 0.0, // blink
            0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, // blink's normals
        ];
        let bin: Vec<u8> = floats.iter().flat_map(|v| v.to_le_bytes()).collect();
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"name":"face","mesh":0,"rotation":[0,{s},0,{s}]}}],
            "meshes":[{{"primitives":[
                {{"attributes":{{"POSITION":0}},"targets":[{{"POSITION":1}},{{"POSITION":2,"NORMAL":3}}]}},
                {{"attributes":{{"POSITION":0}}}}],
              "weights":[0.25,0.5],"extras":{{"targetNames":["smile","blink"]}}}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}},
              {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[0.1,0,0]}},
              {{"bufferView":2,"componentType":5126,"count":3,"type":"VEC3","min":[0,0.2,0],"max":[0,0.2,0]}},
              {{"bufferView":3,"componentType":5126,"count":3,"type":"VEC3"}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}},
                           {{"buffer":0,"byteOffset":36,"byteLength":36}},
                           {{"buffer":0,"byteOffset":72,"byteLength":36}},
                           {{"buffer":0,"byteOffset":108,"byteLength":36}}],
            "buffers":[{{"byteLength":{}}}]}}"#,
            bin.len()
        );
        let model = load_glb(&pack(json, bin)).unwrap();
        let face = model.mesh("face").unwrap();
        assert_eq!(face.mesh.positions.len(), 6);
        assert_eq!(face.morph_weights, [0.25, 0.5]);
        let [smile, blink] = &face.morphs[..] else {
            panic!("two targets, not {}", face.morphs.len());
        };
        assert_eq!(
            (smile.name.as_str(), blink.name.as_str()),
            ("smile", "blink")
        );
        let near = |a: [f32; 3], b: [f32; 3]| Vec3::from(a).abs_diff_eq(Vec3::from(b), 1e-6);
        // Turned as the vertices are: +x becomes −z, +z becomes +x; the second primitive's
        // vertices move under neither.
        assert!(near(smile.positions[1], [0.0, 0.0, -0.1]));
        assert!(
            smile.normals.is_empty(),
            "a target without normals keeps none"
        );
        for v in 0..6 {
            let lifted = if v < 3 { 0.2 } else { 0.0 };
            assert!(near(blink.positions[v], [0.0, lifted, 0.0]));
            assert!(near(blink.normals[v], [lifted * 5.0, 0.0, 0.0]));
            if v != 1 {
                assert_eq!(smile.positions[v], [0.0; 3]);
            }
        }
        // A mesh without targets has none.
        let model = load_glb(&triangle_glb()).unwrap();
        let tri = model.mesh("tri").unwrap();
        assert!(tri.morphs.is_empty() && tri.morph_weights.is_empty());
    }

    /// A textured triangle: UVs, an embedded image, a sampler that clamps u and mirrors v, a
    /// transform on the base colour, and the factors the artist set.
    #[test]
    fn a_textured_material_keeps_what_the_artist_set() {
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0];
        let uvs: [f32; 6] = [0.0, 0.0, 2.0, 0.0, 0.0, 3.0];
        let mut bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        bin.extend(uvs.iter().flat_map(|v| v.to_le_bytes()));
        let image = b"\x89PNG not really".to_vec();
        bin.extend_from_slice(&image);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let half_pi = std::f32::consts::FRAC_PI_2;
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "extensionsUsed":["KHR_texture_transform"],
            "nodes":[{{"name":"tri","mesh":0}}],
            "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"material":0}}]}}],
            "materials":[{{"name":"painted","alphaMode":"MASK",
              "pbrMetallicRoughness":{{"baseColorFactor":[1,0.5,0.25,1],"roughnessFactor":0.7,"metallicFactor":0.2,
                "baseColorTexture":{{"index":0,"extensions":{{"KHR_texture_transform":{{"offset":[0.5,0.25],"rotation":{half_pi},"scale":[2,4]}}}}}},
                "metallicRoughnessTexture":{{"index":0,"texCoord":1}}}},
              "normalTexture":{{"index":0,"scale":0.5}},
              "occlusionTexture":{{"index":0,"strength":0.3}}}}],
            "textures":[{{"source":0,"sampler":0}}],
            "samplers":[{{"wrapS":33071,"wrapT":33648}}],
            "images":[{{"name":"paint","bufferView":2,"mimeType":"image/png"}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,-1],"max":[1,0,0]}},
              {{"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}}],
            "bufferViews":[{{"buffer":0,"byteLength":36}},
                           {{"buffer":0,"byteOffset":36,"byteLength":24}},
                           {{"buffer":0,"byteOffset":60,"byteLength":{}}}],
            "buffers":[{{"byteLength":{}}}]}}"#,
            image.len(),
            bin.len()
        );
        let model = load_glb(&pack(json, bin)).unwrap();
        let tri = model.mesh("tri").unwrap();
        assert_eq!(tri.mesh.uvs, vec![[0.0, 0.0], [2.0, 0.0], [0.0, 3.0]]);
        assert_eq!(model.images.len(), 1);
        assert_eq!(model.images[0].name, "paint");
        assert_eq!(model.images[0].mime, "image/png");
        assert_eq!(model.images[0].bytes, image);
        let m = &tri.materials[0];
        assert_eq!(m.base_color, [1.0, 0.5, 0.25, 1.0]);
        let base = m.base_color_texture.unwrap();
        assert_eq!(base.image, 0);
        assert_eq!(base.wrap, [Wrap::Clamp, Wrap::Mirror]);
        // Scale (2, 4), a quarter turn, then the offset: (1, 0) goes to (0.5, −1.75), (0, 1)
        // to (4.5, 0.25).
        let apply =
            |t: [f32; 6], u: f32, v: f32| [t[0] * u + t[1] * v + t[2], t[3] * u + t[4] * v + t[5]];
        let a = apply(base.transform, 1.0, 0.0);
        let b = apply(base.transform, 0.0, 1.0);
        assert!(
            (a[0] - 0.5).abs() < 1e-5 && (a[1] + 1.75).abs() < 1e-5,
            "{a:?}"
        );
        assert!(
            (b[0] - 4.5).abs() < 1e-5 && (b[1] - 0.25).abs() < 1e-5,
            "{b:?}"
        );
        // The normal and occlusion maps share the base colour's transform; the metallic-
        // roughness map reads a second UV set, which Forge does not draw, and says so.
        assert_eq!(m.normal_texture.unwrap().transform, base.transform);
        assert_eq!(m.normal_scale, 0.5);
        assert_eq!(m.occlusion_strength, 0.3);
        assert!(m.occlusion_texture.is_some());
        assert!(m.metallic_roughness_texture.is_none());
        assert!(m.emissive_texture.is_none());
        // MASK without a cutoff cuts at glTF's default, 0.5; one side only, glTF's default.
        assert_eq!(m.alpha_cutoff, Some(0.5));
        assert!(!m.double_sided);
        assert_eq!(model.unsupported.len(), 1, "{:?}", model.unsupported);
        assert!(model.unsupported.iter().any(|u| u.contains("UV set 1")));
        // A file without UVs gives a mesh without them.
        let plain = load_glb(&triangle_glb()).unwrap();
        assert!(plain.mesh("tri").unwrap().mesh.uvs.is_empty());
        assert!(plain.images.is_empty() && plain.unsupported.is_empty());
    }

    /// A `.gltf` with its buffer and its image in files beside it, their names escaped.
    #[test]
    fn a_gltf_reads_the_files_beside_it() {
        let dir = std::env::temp_dir().join(format!("forge-gltf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0];
        let bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(dir.join("the tri.bin"), &bin).unwrap();
        std::fs::write(dir.join("paint.jpg"), b"\xFF\xD8 not really").unwrap();
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"name":"tri","mesh":0}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],
            "textures":[{"source":0}],
            "images":[{"uri":"paint.jpg"}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,-1],"max":[1,0,0]}],
            "bufferViews":[{"buffer":0,"byteLength":36}],
            "buffers":[{"uri":"the%20tri.bin","byteLength":36}]}"#;
        std::fs::write(dir.join("tri.gltf"), json).unwrap();
        let model = load_gltf(&dir.join("tri.gltf")).unwrap();
        assert_eq!(
            model.mesh("tri").unwrap().mesh.positions[1],
            [1.0, 0.0, 0.0]
        );
        assert_eq!(model.images[0].mime, "image/jpeg");
        assert_eq!(model.images[0].name, "paint.jpg");
        // From memory, the same file has nothing beside it.
        assert!(matches!(
            load_glb(json.as_bytes()),
            Err(GltfError::ExternalBuffer(0))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_is_not_a_model() {
        assert!(load_glb(b"not a model").is_err());
    }
}

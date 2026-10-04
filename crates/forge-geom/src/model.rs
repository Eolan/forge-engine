//! glTF 2.0 models into Forge's meshes (issue #138): the binary form (`.glb`) that Blender and
//! most tools export, read through the `gltf` crate. Each mesh of the scene comes out as a
//! [`TriMesh`] in the scene's frame (its nodes' transforms applied), its primitives as material
//! sections, with the materials' base colour, roughness and metalness to make rows of.
//!
//! glTF is +Y up, metres, and a model's front faces `+Z`; Blender's exporter turns its +Y
//! (forward) into glTF's `−Z`, which is Forge's forward too.

use glam::{Mat3, Mat4, Vec3};

use crate::procedural::TriMesh;
use crate::skin::VertexSkin;

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
    /// A skinned primitive's joints or weights are not one per vertex.
    #[error("mesh {mesh}: not one joint set and one weight set per vertex")]
    BadSkin {
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
}

/// The meshes of a model.
#[derive(Clone, Debug, Default)]
pub struct Model {
    /// In the order the scene's nodes list them.
    pub meshes: Vec<ModelMesh>,
    /// The named nodes that hold no mesh (Blender's empties: a joint's pivot, a socket), with
    /// where they are in the scene's frame (#143).
    pub points: Vec<(String, Vec3)>,
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
/// transforms applied.
pub fn load_glb(bytes: &[u8]) -> Result<Model, GltfError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();
    for buffer in gltf.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(GltfError::ExternalBuffer(buffer.index()));
        }
    }
    let mut model = Model::default();
    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .into_iter();
    for scene in scene {
        for node in scene.nodes() {
            visit(&node, Mat4::IDENTITY, blob, &mut model)?;
        }
    }
    Ok(model)
}

fn visit(
    node: &gltf::Node,
    parent: Mat4,
    blob: Option<&[u8]>,
    model: &mut Model,
) -> Result<(), GltfError> {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        let name = node.name().or(mesh.name()).unwrap_or_default().to_owned();
        // A skinned mesh stays in its bind pose: its joints place it.
        let skinned = node.skin().is_some();
        let frame = if skinned { Mat4::IDENTITY } else { world };
        model
            .meshes
            .push(read_mesh(&mesh, frame, name, skinned, blob)?);
    } else if let Some(name) = node.name() {
        model
            .points
            .push((name.to_owned(), world.transform_point3(Vec3::ZERO)));
    }
    for child in node.children() {
        visit(&child, world, blob, model)?;
    }
    Ok(())
}

fn read_mesh(
    mesh: &gltf::Mesh,
    world: Mat4,
    name: String,
    skinned: bool,
    blob: Option<&[u8]>,
) -> Result<ModelMesh, GltfError> {
    let normal_matrix = Mat3::from_mat4(world).inverse().transpose();
    // Mirrored transforms turn the triangles inside out: wind them back.
    let flip = world.determinant() < 0.0;
    let mut out = TriMesh::default();
    let mut materials: Vec<ModelMaterial> = Vec::new();
    let mut skin: Vec<VertexSkin> = Vec::new();
    let mut needs_normals = false;
    for primitive in mesh.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            return Err(GltfError::NotTriangles { mesh: name });
        }
        let reader = primitive.reader(|_| blob);
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
        let pbr = material.pbr_metallic_roughness();
        let strength = material.emissive_strength().unwrap_or(1.0);
        let this = ModelMaterial {
            name: material.name().unwrap_or_default().to_owned(),
            base_color: pbr.base_color_factor(),
            roughness: pbr.roughness_factor(),
            metallic: pbr.metallic_factor(),
            emissive: material.emissive_factor().map(|c| c * strength),
        };
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
    // One material: no sections needed.
    if materials.len() <= 1 {
        out.sections.clear();
    }
    Ok(ModelMesh {
        name,
        mesh: out,
        materials,
        skin: skinned.then_some(skin),
    })
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
    }

    #[test]
    fn garbage_is_not_a_model() {
        assert!(load_glb(b"not a model").is_err());
    }
}

//! Skinned meshes (#165): a mesh whose vertices follow a skeleton's joints, cooked so a
//! compute pass can move its cluster vertices every frame.
//!
//! The cook differs from [`MeshletMesh::build_with`] in three ways:
//! - **one level:** every cluster of level 0 is a root, always drawn at full detail; the
//!   simplifier's errors are measured on the bind pose and mean nothing on a bent body;
//! - **one sphere for every cluster:** a cluster moves with its joints, so its bounds are the
//!   sphere the whole body stays in whatever its pose (`bound`), and the normal cone is off;
//! - **the cluster vertices' sources:** [`SkinnedMesh::vertices`] gives, in the order the
//!   pages hold them (cluster by cluster), each vertex's bind position, normal, joints and
//!   weights, which the skin pass reads to write the pages.

use bytemuck::{Pod, Zeroable};

use crate::lod;
use crate::meshlet::{MeshletMesh, split_sections};
use crate::page::{self, encode_normal};
use crate::procedural::TriMesh;

/// The joints a vertex follows and how much (glTF's `JOINTS_0` and `WEIGHTS_0`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VertexSkin {
    /// Joint indices, in the skin's order.
    pub joints: [u16; 4],
    /// Their weights, summing to 1.
    pub weights: [f32; 4],
}

/// A cluster vertex as the skin pass reads it (32 bytes; `SkinVertex` in `skin.slang`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SkinVertex {
    /// Bind-pose position.
    pub position: [f32; 3],
    /// Bind-pose normal, octahedral ([`encode_normal`]).
    pub normal: u32,
    /// Four 16-bit joint indices: 0 and 1 in the first word (low, high), 2 and 3 in the second.
    pub joints: [u32; 2],
    /// Four 16-bit unsigned normalised weights, packed as the joints, summing to 65 535.
    pub weights: [u32; 2],
}

impl SkinVertex {
    /// Packs a vertex.
    pub fn new(position: [f32; 3], normal: [f32; 3], skin: VertexSkin) -> Self {
        let pack = |v: [u16; 4]| {
            [
                u32::from(v[0]) | u32::from(v[1]) << 16,
                u32::from(v[2]) | u32::from(v[3]) << 16,
            ]
        };
        Self {
            position,
            normal: encode_normal(normal),
            joints: pack(skin.joints),
            weights: pack(quantize_weights(skin.weights)),
        }
    }

    /// The joint indices.
    pub fn joints(&self) -> [u16; 4] {
        unpack(self.joints)
    }

    /// The weights, as fractions of 65 535.
    pub fn weights(&self) -> [u16; 4] {
        unpack(self.weights)
    }
}

fn unpack(v: [u32; 2]) -> [u16; 4] {
    [
        v[0] as u16,
        (v[0] >> 16) as u16,
        v[1] as u16,
        (v[1] >> 16) as u16,
    ]
}

/// Weights as 16-bit fractions summing to exactly 65 535 (the largest takes the rounding), so
/// a vertex at rest stays where it was bound. Weights summing to 0 go to the first joint.
fn quantize_weights(weights: [f32; 4]) -> [u16; 4] {
    let weights = weights.map(|w| if w.is_finite() { w.max(0.0) } else { 0.0 });
    let sum: f32 = weights.iter().sum();
    if sum <= 0.0 {
        return [u16::MAX, 0, 0, 0];
    }
    let mut q = weights.map(|w| (w / sum * 65535.0).round().min(65535.0) as u16);
    let total: i32 = q.iter().map(|&w| i32::from(w)).sum();
    let largest = (0..4).max_by_key(|&k| q[k]).expect("four weights");
    q[largest] = (i32::from(q[largest]) + 65535 - total).clamp(0, 65535) as u16;
    q
}

/// A cooked skinned mesh: its clusters (one level, all roots) and their vertices' skin data.
pub struct SkinnedMesh {
    /// The clusters and their pages, the vertices in the bind pose.
    pub mesh: MeshletMesh,
    /// Per cluster vertex, in the pages' order (cluster by cluster, each cluster's vertices in
    /// its payload's order), what the skin pass moves it from.
    pub vertices: Vec<SkinVertex>,
}

impl SkinnedMesh {
    /// Cuts `mesh` into clusters for skinning, `skin` giving each of its vertices' joints.
    /// `bound` (centre and radius, in the frame the skinning matrices take the vertices to)
    /// is a sphere the mesh stays in whatever its pose: every cluster's and the mesh's.
    ///
    /// # Panics
    ///
    /// When `skin` does not have one entry per vertex of `mesh`.
    pub fn cook(mesh: &TriMesh, skin: &[VertexSkin], bound: ([f32; 3], f32)) -> Self {
        assert_eq!(skin.len(), mesh.positions.len(), "one skin per vertex");
        let (vertices, indices, vertex_section, source) = split_sections(mesh);
        let indices = meshopt::optimize_vertex_cache(&indices, vertices.len());
        let mut dag = lod::build_dag(&indices, &vertices, &vertex_section, 0.0, 1);
        let (center, radius) = bound;
        for m in &mut dag.meshlets {
            m.center = center;
            m.radius = radius;
            m.self_center = center;
            m.self_radius = radius;
            m.parent_center = center;
            m.parent_radius = radius;
            // No normal cone: the faces turn with the joints.
            m.cone_apex = [0.0; 3];
            m.cone_axis = [0.0; 3];
            m.cone_cutoff = 1.0;
        }
        let pages = page::pack(&mut dag, &vertices);
        let mut skinned = Vec::with_capacity(dag.meshlet_vertices.len());
        for (m, range) in dag.meshlets.iter().zip(&dag.ranges) {
            let first = range.vertex_offset as usize;
            for &v in &dag.meshlet_vertices[first..first + m.vertex_count as usize] {
                let s = source[v as usize] as usize;
                skinned.push(SkinVertex::new(mesh.positions[s], mesh.normals[s], skin[s]));
            }
        }
        Self {
            mesh: MeshletMesh {
                meshlets: dag.meshlets,
                page_count: pages.count(),
                pages: pages.bytes,
                root_pages: pages.root_pages,
                page_file: None,
                triangle_count: indices.len() / 3,
                dag_triangle_count: dag.triangle_count,
                clusters_per_level: dag.clusters_per_level,
                center,
                radius,
            },
            vertices: skinned,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedural::asteroid;
    use forge_core::Seed;

    /// An asteroid skinned to two joints by height, blended across the equator, and split
    /// into two material sections (so some vertices are copied).
    fn skinned_rock() -> (TriMesh, Vec<VertexSkin>) {
        let mut mesh = asteroid(Seed::new(7), 24, 1.0, 0.2);
        mesh.sections = mesh
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| u8::from(mesh.positions[t[0] as usize][0] > 0.0))
            .collect();
        let skin = mesh
            .positions
            .iter()
            .map(|p| {
                let up = (p[1] * 2.0 + 0.5).clamp(0.0, 1.0);
                VertexSkin {
                    joints: [0, 1, 0, 0],
                    weights: [1.0 - up, up, 0.0, 0.0],
                }
            })
            .collect();
        (mesh, skin)
    }

    #[test]
    fn every_cluster_is_a_root_bounded_by_the_pose_sphere() {
        let (mesh, skin) = skinned_rock();
        let cooked = SkinnedMesh::cook(&mesh, &skin, ([0.0, 0.5, 0.0], 3.0));
        let m = &cooked.mesh;
        assert_eq!(m.levels(), 1);
        assert!(m.meshlets.len() > 4);
        assert_eq!(m.root_pages, m.page_count);
        for c in &m.meshlets {
            assert!(c.parent_error.is_infinite() && c.self_error == 0.0);
            assert_eq!((c.center, c.radius), ([0.0, 0.5, 0.0], 3.0));
            assert_eq!(c.cone_cutoff, 1.0);
        }
        assert_eq!(m.triangle_count, mesh.triangle_count());
    }

    #[test]
    fn each_cluster_vertex_carries_its_source_s_skin() {
        let (mesh, skin) = skinned_rock();
        let cooked = SkinnedMesh::cook(&mesh, &skin, ([0.0; 3], 3.0));
        let m = &cooked.mesh;
        let mut k = 0;
        for c in &m.meshlets {
            for i in 0..c.vertex_count as usize {
                let paged = m.vertex(c, i);
                let s = &cooked.vertices[k];
                // The page and the skin data hold the same vertex at the same place.
                assert_eq!(paged.position, s.position);
                assert_eq!(paged.normal, s.normal);
                // Its weights are its source's: the height says how much of joint 1.
                let up = (s.position[1] * 2.0 + 0.5).clamp(0.0, 1.0);
                let w = s.weights();
                assert_eq!(s.joints(), [0, 1, 0, 0]);
                assert!((f32::from(w[1]) / 65535.0 - up).abs() < 1e-4);
                assert_eq!(w.iter().map(|&w| u32::from(w)).sum::<u32>(), 65535);
                k += 1;
            }
        }
        assert_eq!(k, cooked.vertices.len());
    }

    #[test]
    fn weights_quantize_to_a_whole() {
        assert_eq!(quantize_weights([1.0, 0.0, 0.0, 0.0]), [65535, 0, 0, 0]);
        assert_eq!(quantize_weights([0.0; 4]), [65535, 0, 0, 0]);
        let q = quantize_weights([0.3333, 0.3333, 0.3334, 0.0]);
        assert_eq!(q.iter().map(|&w| u32::from(w)).sum::<u32>(), 65535);
        // Unnormalised weights are normalised.
        assert_eq!(quantize_weights([2.0, 2.0, 0.0, 0.0]), [32768, 32767, 0, 0]);
    }
}

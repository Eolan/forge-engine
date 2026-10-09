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
//!   weights, which the skin pass reads to write the pages;
//! - **its morph targets** (#169, [`SkinnedMesh::cook_morphed`]): per cluster vertex, the
//!   changes the targets that move it make, which the skin pass adds by the frame's weights
//!   before the joints move the vertex;
//! - **eight joints a vertex** (#169, [`SkinnedMesh::cook_full`]): a rig whose vertices follow
//!   up to eight keeps their four heaviest in [`SkinnedMesh::vertices`] and the others in
//!   [`SkinnedMesh::more`].

use bytemuck::{Pod, Zeroable};

use crate::lod;
use crate::meshlet::{GpuMeshlet, MeshletMesh, split_sections};
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
        Self {
            position,
            normal: encode_normal(normal),
            joints: pack(skin.joints),
            weights: pack(quantize_weights(skin.weights)),
        }
    }

    /// Packs a vertex that follows up to eight joints (#169): `skin` and `more` (glTF's
    /// `JOINTS_1` and `WEIGHTS_1`), their eight weights summing to 1. The four heaviest go in
    /// the vertex, the others in the returned [`SkinMore`]; the eight quantised weights sum to
    /// 65 535.
    pub fn new_eight(
        position: [f32; 3],
        normal: [f32; 3],
        skin: VertexSkin,
        more: VertexSkin,
    ) -> (Self, SkinMore) {
        let mut pairs: [(u16, f32); 8] = std::array::from_fn(|k| match k {
            0..4 => (skin.joints[k], skin.weights[k]),
            _ => (more.joints[k - 4], more.weights[k - 4]),
        });
        pairs.sort_by(|a, b| b.1.total_cmp(&a.1));
        let q = quantize_weights(pairs.map(|(_, w)| w));
        let joints = |range: std::ops::Range<usize>| -> [u16; 4] {
            std::array::from_fn(|k| pairs[range.start + k].0)
        };
        let weights = |range: std::ops::Range<usize>| -> [u16; 4] {
            std::array::from_fn(|k| q[range.start + k])
        };
        (
            Self {
                position,
                normal: encode_normal(normal),
                joints: pack(joints(0..4)),
                weights: pack(weights(0..4)),
            },
            SkinMore {
                joints: pack(joints(4..8)),
                weights: pack(weights(4..8)),
            },
        )
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

/// A vertex's second four joints and weights as the skin pass reads them (16 bytes; `SkinMore`
/// in `skin.slang`), for a mesh whose vertices follow up to eight joints (#169): packed as
/// [`SkinVertex`]'s, the vertex's eight weights together summing to 65 535.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SkinMore {
    /// Joints 4 to 7, packed as [`SkinVertex::joints`].
    pub joints: [u32; 2],
    /// Their weights, packed as [`SkinVertex::weights`].
    pub weights: [u32; 2],
}

impl SkinMore {
    /// The joint indices.
    pub fn joints(&self) -> [u16; 4] {
        unpack(self.joints)
    }

    /// The weights, as fractions of 65 535.
    pub fn weights(&self) -> [u16; 4] {
        unpack(self.weights)
    }
}

fn pack(v: [u16; 4]) -> [u32; 2] {
    [
        u32::from(v[0]) | u32::from(v[1]) << 16,
        u32::from(v[2]) | u32::from(v[3]) << 16,
    ]
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
fn quantize_weights<const N: usize>(weights: [f32; N]) -> [u16; N] {
    let weights = weights.map(|w| if w.is_finite() { w.max(0.0) } else { 0.0 });
    let sum: f32 = weights.iter().sum();
    if sum <= 0.0 {
        let mut first = [0; N];
        first[0] = u16::MAX;
        return first;
    }
    let mut q = weights.map(|w| (w / sum * 65535.0).round().min(65535.0) as u16);
    let total: i32 = q.iter().map(|&w| i32::from(w)).sum();
    let largest = (0..N).max_by_key(|&k| q[k]).expect("some weights");
    q[largest] = (i32::from(q[largest]) + 65535 - total).clamp(0, 65535) as u16;
    q
}

/// A morph target (glTF's, a blend shape, #169): per vertex of its mesh, how far the target at
/// its full weight moves the vertex's bind position and normal. The skin pass adds the targets,
/// each times its weight, before the joints move the vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MorphTarget {
    /// Its name (the file's `targetNames`), or empty.
    pub name: String,
    /// Per vertex, the position's change.
    pub positions: Vec<[f32; 3]>,
    /// Per vertex, the normal's change; empty when the target leaves the normals.
    pub normals: Vec<[f32; 3]>,
}

/// A vertex's change under one morph target as the skin pass reads it (32 bytes;
/// `MorphDelta` in `skin.slang`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MorphDelta {
    /// The position's change at the target's full weight.
    pub position: [f32; 3],
    /// The target, in its mesh's order (the scene rebases it into the frame's weights).
    pub target: u32,
    /// The normal's change at the target's full weight.
    pub normal: [f32; 3],
    /// Zero.
    pub pad: u32,
}

/// A skinned mesh's morph targets, cooked for the skin pass: only the vertices a target moves
/// keep a delta for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Morphs {
    /// How many targets, so how many weights a frame.
    pub targets: u32,
    /// Per cluster vertex (as [`SkinnedMesh::vertices`]), its first delta and how many.
    pub ranges: Vec<[u32; 2]>,
    /// The deltas, vertex by vertex.
    pub deltas: Vec<MorphDelta>,
}

impl Morphs {
    /// What the targets at `weights` add to cluster vertex `vertex`'s bind position and normal,
    /// as the skin pass adds them.
    pub fn offset(&self, vertex: usize, weights: &[f32]) -> ([f32; 3], [f32; 3]) {
        let [first, count] = self.ranges[vertex];
        let (mut position, mut normal) = ([0.0; 3], [0.0; 3]);
        for d in &self.deltas[first as usize..(first + count) as usize] {
            let w = weights[d.target as usize];
            for k in 0..3 {
                position[k] += w * d.position[k];
                normal[k] += w * d.normal[k];
            }
        }
        (position, normal)
    }
}

/// A cooked skinned mesh: its clusters (one level, all roots) and their vertices' skin data.
pub struct SkinnedMesh {
    /// The clusters and their pages, the vertices in the bind pose.
    pub mesh: MeshletMesh,
    /// Per cluster vertex, in the pages' order (cluster by cluster, each cluster's vertices in
    /// its payload's order), what the skin pass moves it from.
    pub vertices: Vec<SkinVertex>,
    /// Its morph targets (#169), when it has any.
    pub morphs: Option<Morphs>,
    /// Per cluster vertex as `vertices`, its second four joints, for a mesh whose vertices
    /// follow up to eight (#169).
    pub more: Option<Vec<SkinMore>>,
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
        Self::cook_morphed(mesh, skin, &[], bound)
    }

    /// [`Self::cook`] with morph targets (#169), each giving a change per vertex of `mesh`.
    /// The bound grows by the farthest the targets together move a vertex, their weights
    /// taken between 0 and 1.
    ///
    /// # Panics
    ///
    /// When `skin` or a target does not have one entry per vertex of `mesh`.
    pub fn cook_morphed(
        mesh: &TriMesh,
        skin: &[VertexSkin],
        targets: &[MorphTarget],
        bound: ([f32; 3], f32),
    ) -> Self {
        Self::cook_full(mesh, skin, &[], targets, bound)
    }

    /// [`Self::cook_morphed`] for vertices that follow up to eight joints (#169): `more` gives
    /// each vertex's second four (glTF's `JOINTS_1` and `WEIGHTS_1`), or is empty for four.
    /// A vertex's four heaviest joints go in [`Self::vertices`], the others in [`Self::more`].
    ///
    /// # Panics
    ///
    /// When `skin`, `more` (if not empty) or a target does not have one entry per vertex of
    /// `mesh`.
    pub fn cook_full(
        mesh: &TriMesh,
        skin: &[VertexSkin],
        more: &[VertexSkin],
        targets: &[MorphTarget],
        bound: ([f32; 3], f32),
    ) -> Self {
        assert_eq!(skin.len(), mesh.positions.len(), "one skin per vertex");
        assert!(
            more.is_empty() || more.len() == skin.len(),
            "none or one more per vertex"
        );
        for t in targets {
            assert_eq!(
                t.positions.len(),
                mesh.positions.len(),
                "one change per vertex"
            );
            assert!(t.normals.is_empty() || t.normals.len() == mesh.positions.len());
        }
        let reach = (0..mesh.positions.len())
            .map(|v| {
                targets
                    .iter()
                    .map(|t| t.positions[v].iter().map(|c| c * c).sum::<f32>().sqrt())
                    .sum::<f32>()
            })
            .fold(0.0, f32::max);
        let (center, radius) = (bound.0, bound.1 + reach);
        Self::cook_with(mesh, skin, more, targets, (center, radius), |m| {
            m.center = center;
            m.radius = radius;
            m.self_center = center;
            m.self_radius = radius;
            m.parent_center = center;
            m.parent_radius = radius;
        })
    }

    /// Cuts `mesh`, a surface a height field raises and lowers (#185's deformable ground), into
    /// clusters the skin pass moves: every vertex on joint 0, which places the mesh, and each
    /// cluster bounded by its own sphere grown by `reach`, the farthest the field moves a
    /// vertex, so the culls still cut a large ground to what is seen.
    pub fn cook_displaced(mesh: &TriMesh, reach: f32) -> Self {
        let skin = vec![
            VertexSkin {
                joints: [0; 4],
                weights: [1.0, 0.0, 0.0, 0.0],
            };
            mesh.positions.len()
        ];
        let (mut low, mut high) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &mesh.positions {
            for k in 0..3 {
                low[k] = low[k].min(p[k]);
                high[k] = high[k].max(p[k]);
            }
        }
        let center: [f32; 3] = std::array::from_fn(|k| 0.5 * (low[k] + high[k]));
        let radius = mesh
            .positions
            .iter()
            .map(|p| {
                (0..3)
                    .map(|k| (p[k] - center[k]).powi(2))
                    .sum::<f32>()
                    .sqrt()
            })
            .fold(0.0, f32::max);
        Self::cook_with(mesh, &skin, &[], &[], (center, radius + reach), |m| {
            m.radius += reach;
            m.self_center = m.center;
            m.self_radius = m.radius;
            m.parent_center = m.center;
            m.parent_radius = m.radius;
        })
    }

    /// The cook: `bound` is the mesh's sphere, `cluster` sets each cluster's bounds.
    fn cook_with(
        mesh: &TriMesh,
        skin: &[VertexSkin],
        more: &[VertexSkin],
        targets: &[MorphTarget],
        bound: ([f32; 3], f32),
        cluster: impl Fn(&mut GpuMeshlet),
    ) -> Self {
        let (vertices, indices, vertex_section, source) = split_sections(mesh);
        let indices = meshopt::optimize_vertex_cache(&indices, vertices.len());
        let mut dag = lod::build_dag(&indices, &vertices, &vertex_section, 0.0, 1);
        let (center, radius) = bound;
        for m in &mut dag.meshlets {
            cluster(m);
            // No normal cone: the faces turn with the joints.
            m.cone_apex = [0.0; 3];
            m.cone_axis = [0.0; 3];
            m.cone_cutoff = 1.0;
        }
        let uvs = !mesh.uvs.is_empty();
        let pages = page::pack(&mut dag, &vertices, uvs);
        let mut skinned = Vec::with_capacity(dag.meshlet_vertices.len());
        let mut extra = Vec::new();
        let mut morphs = Morphs {
            targets: targets.len() as u32,
            ..Morphs::default()
        };
        for (m, range) in dag.meshlets.iter().zip(&dag.ranges) {
            let first = range.vertex_offset as usize;
            for &v in &dag.meshlet_vertices[first..first + m.vertex_count as usize] {
                let s = source[v as usize] as usize;
                if more.is_empty() {
                    skinned.push(SkinVertex::new(mesh.positions[s], mesh.normals[s], skin[s]));
                } else {
                    let (vertex, rest) =
                        SkinVertex::new_eight(mesh.positions[s], mesh.normals[s], skin[s], more[s]);
                    skinned.push(vertex);
                    extra.push(rest);
                }
                if targets.is_empty() {
                    continue;
                }
                let start = morphs.deltas.len() as u32;
                for (t, target) in targets.iter().enumerate() {
                    let position = target.positions[s];
                    let normal = target.normals.get(s).copied().unwrap_or([0.0; 3]);
                    if position != [0.0; 3] || normal != [0.0; 3] {
                        morphs.deltas.push(MorphDelta {
                            position,
                            target: t as u32,
                            normal,
                            pad: 0,
                        });
                    }
                }
                morphs
                    .ranges
                    .push([start, morphs.deltas.len() as u32 - start]);
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
                uvs,
            },
            vertices: skinned,
            morphs: (!targets.is_empty()).then_some(morphs),
            more: (!more.is_empty()).then_some(extra),
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
    fn each_cluster_vertex_carries_the_changes_its_source_s_targets_make() {
        let (mesh, skin) = skinned_rock();
        // One target swells the upper half by a tenth along x, one lifts every vertex 5 cm and
        // tilts its normal.
        let swell = MorphTarget {
            name: "swell".into(),
            positions: mesh
                .positions
                .iter()
                .map(|p| {
                    if p[1] > 0.0 {
                        [0.1 * p[0], 0.0, 0.0]
                    } else {
                        [0.0; 3]
                    }
                })
                .collect(),
            normals: Vec::new(),
        };
        let lift = MorphTarget {
            name: "lift".into(),
            positions: vec![[0.0, 0.05, 0.0]; mesh.positions.len()],
            normals: vec![[0.0, 0.0, 0.2]; mesh.positions.len()],
        };
        let cooked = SkinnedMesh::cook_morphed(&mesh, &skin, &[swell, lift], ([0.0; 3], 3.0));
        let morphs = cooked.morphs.as_ref().expect("morphed");
        assert_eq!(morphs.targets, 2);
        assert_eq!(morphs.ranges.len(), cooked.vertices.len());
        // The farthest a vertex moves, both targets at their full weight, grows the bound.
        let widest = mesh
            .positions
            .iter()
            .map(|p| p[0].abs())
            .fold(0.0, f32::max);
        assert!(cooked.mesh.radius > 3.05 && cooked.mesh.radius <= 3.05 + 0.1 * widest + 1e-5);
        for (k, s) in cooked.vertices.iter().enumerate() {
            let p = s.position;
            let (moved, turned) = morphs.offset(k, &[1.0, 0.5]);
            let swell = if p[1] > 0.0 { 0.1 * p[0] } else { 0.0 };
            assert_eq!(moved, [swell, 0.025, 0.0]);
            assert_eq!(turned, [0.0, 0.0, 0.1]);
            // Only the changes a target makes are kept.
            let kept = morphs.ranges[k][1];
            assert_eq!(kept, if swell != 0.0 { 2 } else { 1 });
        }
        // Without targets, nothing.
        assert!(
            SkinnedMesh::cook(&mesh, &skin, ([0.0; 3], 3.0))
                .morphs
                .is_none()
        );
    }

    #[test]
    fn a_vertex_on_eight_joints_keeps_its_four_heaviest_first_and_all_eight_weights() {
        let (mesh, _) = skinned_rock();
        // Every vertex on joints 0 to 7, weighted by its height: joint 7 heaviest at the top,
        // joint 0 at the bottom.
        let weights = |p: [f32; 3]| -> [f32; 8] {
            let up = (p[1] + 1.0).clamp(0.0, 2.0) / 2.0;
            let raw: [f32; 8] = std::array::from_fn(|k| 1.0 - 0.9 * (k as f32 / 7.0 - up).abs());
            let sum: f32 = raw.iter().sum();
            raw.map(|w| w / sum)
        };
        let (skin, more): (Vec<VertexSkin>, Vec<VertexSkin>) = mesh
            .positions
            .iter()
            .map(|&p| {
                let w = weights(p);
                (
                    VertexSkin {
                        joints: [0, 1, 2, 3],
                        weights: [w[0], w[1], w[2], w[3]],
                    },
                    VertexSkin {
                        joints: [4, 5, 6, 7],
                        weights: [w[4], w[5], w[6], w[7]],
                    },
                )
            })
            .unzip();
        let cooked = SkinnedMesh::cook_full(&mesh, &skin, &more, &[], ([0.0; 3], 3.0));
        let rest = cooked.more.as_ref().expect("eight joints");
        assert_eq!(rest.len(), cooked.vertices.len());
        for (v, r) in cooked.vertices.iter().zip(rest) {
            let w = weights(v.position);
            let joints: Vec<u16> = v.joints().into_iter().chain(r.joints()).collect();
            let q: Vec<u16> = v.weights().into_iter().chain(r.weights()).collect();
            // All eight joints, each once, at its own weight; the four heaviest first.
            let mut sorted = joints.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, [0, 1, 2, 3, 4, 5, 6, 7]);
            for (j, q) in joints.iter().zip(&q) {
                assert!((f32::from(*q) / 65535.0 - w[usize::from(*j)]).abs() < 1e-4);
            }
            assert!(q[..4].iter().min() >= q[4..].iter().max());
            assert_eq!(q.iter().map(|&w| u32::from(w)).sum::<u32>(), 65535);
        }
        // With four joints, none more.
        assert!(
            SkinnedMesh::cook(&mesh, &skin, ([0.0; 3], 3.0))
                .more
                .is_none()
        );
    }

    #[test]
    fn a_displaced_ground_s_clusters_keep_their_own_spheres_grown_by_its_reach() {
        // A flat grid of 40 by 30 cells, 5 cm each.
        let (nx, nz, cell) = (41u32, 31u32, 0.05f32);
        let mut mesh = TriMesh::default();
        for z in 0..nz {
            for x in 0..nx {
                mesh.positions.push([x as f32 * cell, 0.0, z as f32 * cell]);
                mesh.normals.push([0.0, 1.0, 0.0]);
            }
        }
        for z in 0..nz - 1 {
            for x in 0..nx - 1 {
                let v = z * nx + x;
                mesh.indices
                    .extend_from_slice(&[v, v + nx, v + 1, v + 1, v + nx, v + nx + 1]);
            }
        }
        let reach = 0.1;
        let cooked = SkinnedMesh::cook_displaced(&mesh, reach);
        let m = &cooked.mesh;
        assert_eq!(m.levels(), 1);
        assert!(m.meshlets.len() > 10);
        let mut k = 0;
        for c in &m.meshlets {
            assert_eq!(c.cone_cutoff, 1.0);
            assert!(
                c.radius < 0.8 * m.radius,
                "a cluster's own sphere, not the ground's"
            );
            for _ in 0..c.vertex_count {
                let v = &cooked.vertices[k];
                assert_eq!((v.joints(), v.weights()), ([0; 4], [65535, 0, 0, 0]));
                // Raised or lowered by the reach, the vertex stays in its cluster's sphere.
                for dy in [-reach, reach] {
                    let d: f32 = (0..3)
                        .map(|i| {
                            let p = v.position[i] + if i == 1 { dy } else { 0.0 };
                            (p - c.center[i]).powi(2)
                        })
                        .sum::<f32>()
                        .sqrt();
                    assert!(d <= c.radius + 1e-5);
                }
                k += 1;
            }
        }
        // The ground's, 2 m by 1.5 m: half its diagonal and the reach.
        assert!((m.radius - 1.25 - reach).abs() < 1e-4);
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

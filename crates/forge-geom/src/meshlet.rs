//! Meshlet building with meshoptimizer.
//!
//! A meshlet is a cluster of at most 64 vertices and 124 triangles with a bounding sphere, a
//! normal cone and its place in the cluster LOD DAG (`lod.rs`), the unit of work of the
//! task/mesh pipeline (`shaders/meshlet.slang`). The sizes follow NVIDIA's mesh-shader
//! guidance and the limits reported by the device.

use bytemuck::{Pod, Zeroable};

use crate::lod;
use crate::procedural::TriMesh;

/// Maximum vertices per meshlet.
pub const MESHLET_MAX_VERTICES: usize = 64;
/// Maximum triangles per meshlet.
pub const MESHLET_MAX_TRIANGLES: usize = 124;

/// Vertex layout shared with the shaders (32 bytes, std430).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuVertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Padding.
    pub pad0: f32,
    /// Object-space normal.
    pub normal: [f32; 3],
    /// Padding.
    pub pad1: f32,
}

/// Meshlet record shared with the shaders (112 bytes, natural layout).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuMeshlet {
    /// Bounding sphere centre (culling).
    pub center: [f32; 3],
    /// Bounding sphere radius (culling).
    pub radius: f32,
    /// Normal cone apex.
    pub cone_apex: [f32; 3],
    /// Normal cone cutoff (`cos(angle)`; 1 = wider than a hemisphere, axis zero, no test).
    pub cone_cutoff: f32,
    /// Normal cone axis.
    pub cone_axis: [f32; 3],
    /// First entry in the meshlet vertex index list.
    pub vertex_offset: u32,
    /// First byte in the meshlet triangle list.
    pub triangle_offset: u32,
    /// Vertices in this meshlet.
    pub vertex_count: u32,
    /// Triangles in this meshlet.
    pub triangle_count: u32,
    /// Padding.
    pub pad: u32,
    /// LOD: centre of the sphere of the group this cluster was produced from.
    pub self_center: [f32; 3],
    /// LOD: its radius.
    pub self_radius: f32,
    /// LOD: centre of the sphere of the group that simplified this cluster away.
    pub parent_center: [f32; 3],
    /// LOD: its radius.
    pub parent_radius: f32,
    /// LOD: error (object-space length) of the simplification that produced this cluster;
    /// 0 for level 0.
    pub self_error: f32,
    /// LOD: error of the simplification that consumed it; infinite for a root.
    pub parent_error: f32,
    /// LOD level (0 = full detail).
    pub lod_level: u32,
    /// Padding.
    pub pad2: u32,
}

/// How a mesh is cooked.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CookOptions {
    /// Weight of the vertex normals in the simplification error, in metres of error per unit
    /// of normal change: 0 counts geometry only. A facade's window recess is shallow (a
    /// quarter metre) but turns the normal by 90°: with geometric error alone the coarse
    /// levels lose the windows while they are still several pixels wide, and the vertices
    /// left keep normals that no longer match the surface. With a weight of 1 a 90° turn
    /// counts like about a metre, so windows stay until they are about a pixel.
    pub normal_weight: f32,
}

/// The shape of a cooked DAG (the metrics the research asks to track: roots reached, fill).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DagStats {
    /// LOD levels.
    pub levels: usize,
    /// Clusters over all levels.
    pub clusters: usize,
    /// Clusters no coarser level replaces: 1 when the DAG simplified the mesh down to one
    /// cluster; more when groups stalled (see `lod::STALL_RATIO`).
    pub roots: usize,
    /// Mean triangles per cluster over the maximum (1 = every cluster full).
    pub fill: f64,
}

/// A mesh cut into clusters at every LOD level, ready for upload.
pub struct MeshletMesh {
    /// Vertex buffer (shared by every level).
    pub vertices: Vec<GpuVertex>,
    /// Meshlet records, all levels.
    pub meshlets: Vec<GpuMeshlet>,
    /// Indices into `vertices`, per meshlet.
    pub meshlet_vertices: Vec<u32>,
    /// Local triangle indices (3 bytes each), padded to a multiple of 4.
    pub meshlet_triangles: Vec<u8>,
    /// Triangles at full detail (level 0).
    pub triangle_count: usize,
    /// Triangles over all levels.
    pub dag_triangle_count: usize,
    /// Clusters per LOD level.
    pub clusters_per_level: Vec<u32>,
    /// Bounding sphere centre of the whole mesh.
    pub center: [f32; 3],
    /// Bounding sphere radius of the whole mesh.
    pub radius: f32,
}

impl MeshletMesh {
    /// Optimises the index buffer for the vertex cache, clusters it and builds the LOD DAG,
    /// with the default [`CookOptions`] (geometric error only).
    pub fn build(mesh: &TriMesh) -> Self {
        Self::build_with(mesh, CookOptions::default())
    }

    /// [`MeshletMesh::build`] with explicit options.
    pub fn build_with(mesh: &TriMesh, options: CookOptions) -> Self {
        let vertices: Vec<GpuVertex> = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .map(|(p, n)| GpuVertex {
                position: *p,
                pad0: 0.0,
                normal: *n,
                pad1: 0.0,
            })
            .collect();
        let indices = meshopt::optimize_vertex_cache(&mesh.indices, vertices.len());
        let dag = lod::build_dag(&indices, &vertices, options.normal_weight);
        let (center, radius) = bounding_sphere(&mesh.positions);
        Self {
            vertices,
            meshlets: dag.meshlets,
            meshlet_vertices: dag.meshlet_vertices,
            meshlet_triangles: dag.meshlet_triangles,
            triangle_count: indices.len() / 3,
            dag_triangle_count: dag.triangle_count,
            clusters_per_level: dag.clusters_per_level,
            center,
            radius,
        }
    }

    /// Number of LOD levels.
    pub fn levels(&self) -> usize {
        self.clusters_per_level.len()
    }

    /// The DAG's shape: levels, roots and how full the clusters are.
    pub fn dag_stats(&self) -> DagStats {
        let clusters = self.meshlets.len().max(1);
        DagStats {
            levels: self.levels(),
            clusters: self.meshlets.len(),
            roots: self
                .meshlets
                .iter()
                .filter(|m| m.parent_error.is_infinite())
                .count(),
            fill: self.dag_triangle_count as f64 / (clusters * MESHLET_MAX_TRIANGLES) as f64,
        }
    }
}

fn bounding_sphere(positions: &[[f32; 3]]) -> ([f32; 3], f32) {
    if positions.is_empty() {
        return ([0.0; 3], 0.0);
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in positions {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let radius = positions
        .iter()
        .map(|p| {
            ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2))
                .sqrt()
        })
        .fold(0.0_f32, f32::max);
    (center, radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::Seed;

    fn level0<'a>(built: &'a MeshletMesh) -> impl Iterator<Item = &'a GpuMeshlet> + 'a {
        built.meshlets.iter().filter(|m| m.lod_level == 0)
    }

    #[test]
    fn meshlets_cover_every_triangle_within_limits() {
        let mesh = crate::procedural::asteroid(Seed::new(7), 24, 1.0, 0.2);
        let built = MeshletMesh::build(&mesh);
        assert_eq!(built.triangle_count, mesh.indices.len() / 3);
        let total: usize = level0(&built).map(|m| m.triangle_count as usize).sum();
        assert_eq!(total, built.triangle_count);
        for m in &built.meshlets {
            assert!(m.vertex_count as usize <= MESHLET_MAX_VERTICES);
            assert!(m.triangle_count as usize <= MESHLET_MAX_TRIANGLES);
            assert!(m.radius > 0.0);
            assert!(
                (m.triangle_offset as usize + m.triangle_count as usize * 3)
                    <= built.meshlet_triangles.len()
            );
            for t in 0..m.triangle_count as usize {
                for k in 0..3 {
                    let local =
                        built.meshlet_triangles[m.triangle_offset as usize + t * 3 + k] as u32;
                    assert!(local < m.vertex_count);
                    let vertex = built.meshlet_vertices[(m.vertex_offset + local) as usize];
                    assert!((vertex as usize) < built.vertices.len());
                }
            }
        }
        assert_eq!(built.meshlet_triangles.len() % 4, 0);
    }

    /// The DAG reaches a root, halves per level, and its errors and spheres are monotonic
    /// with every group's clusters sharing the group's values.
    #[test]
    fn the_lod_dag_is_monotonic_and_reaches_a_root() {
        let mesh = crate::procedural::asteroid(Seed::new(700), 48, 1.0, 0.45);
        let built = MeshletMesh::build(&mesh);
        eprintln!(
            "levels: {:?}, dag triangles {} for {} leaf",
            built.clusters_per_level, built.dag_triangle_count, built.triangle_count
        );
        assert!(built.levels() >= 4, "levels {:?}", built.clusters_per_level);
        assert_eq!(
            *built.clusters_per_level.last().unwrap(),
            1,
            "one root cluster"
        );
        // Triangles halve per level; cluster counts shrink a little less (level-0 clusters
        // are not full, simplified ones are).
        for pair in built.clusters_per_level.windows(2) {
            assert!(
                pair[1] < pair[0] && pair[1] as f32 <= pair[0] as f32 * 0.75 + 2.0,
                "levels must shrink: {pair:?}"
            );
        }
        let roots = built
            .meshlets
            .iter()
            .filter(|m| m.parent_error.is_infinite())
            .count();
        assert!(roots >= 1);
        for m in &built.meshlets {
            assert!(
                m.parent_error >= m.self_error,
                "parent error {} < self error {}",
                m.parent_error,
                m.self_error
            );
            if m.parent_error.is_finite() {
                // The parent sphere contains the self sphere.
                let d = (0..3)
                    .map(|i| (m.parent_center[i] - m.self_center[i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                assert!(
                    d + m.self_radius <= m.parent_radius + 1e-3,
                    "parent sphere does not contain the child's"
                );
            }
            if m.lod_level == 0 {
                assert_eq!(m.self_error, 0.0);
            } else {
                assert!(m.self_error > 0.0);
            }
        }
        // Siblings (same parent values) exist: at least one parent sphere is shared by > 1 cluster.
        let mut shared = std::collections::HashMap::new();
        for m in built.meshlets.iter().filter(|m| m.parent_error.is_finite()) {
            *shared.entry(m.parent_center.map(f32::to_bits)).or_insert(0) += 1;
        }
        assert!(shared.values().any(|&n| n > 1));
        // Total DAG size is about twice the leaves.
        assert!(built.dag_triangle_count < built.triangle_count * 3);
    }

    /// meshoptimizer marks a cone wider than a hemisphere with `cone_cutoff == 1` and leaves
    /// the axis at zero; a shader that normalises that axis gets NaN and culls the meshlet
    /// (the bug that punched holes in every rough asteroid). The convention must hold so the
    /// shader can skip the test on `cutoff >= 1`.
    #[test]
    fn wide_cones_are_marked_by_a_unit_cutoff() {
        let mesh = crate::procedural::asteroid(Seed::new(700), 48, 1.0, 0.45);
        let built = MeshletMesh::build(&mesh);
        let wide = level0(&built).filter(|m| m.cone_cutoff >= 1.0).count();
        assert!(wide > 0, "a rough asteroid should have some wide cones");
        for m in &built.meshlets {
            let axis_len = glam::Vec3::from(m.cone_axis).length();
            if m.cone_cutoff >= 1.0 {
                assert!(
                    axis_len < 1e-6 || axis_len > 0.99,
                    "axis {:?} cutoff {}",
                    m.cone_axis,
                    m.cone_cutoff
                );
            } else {
                assert!(
                    (axis_len - 1.0).abs() < 1e-3,
                    "axis {:?} cutoff {}",
                    m.cone_axis,
                    m.cone_cutoff
                );
            }
        }
    }

    /// A culled meshlet must not contain a single front-facing triangle, for either cone test
    /// (apex form and sphere form, as documented by meshoptimizer), evaluated exactly as the
    /// shader does (plain normalisation, test skipped on `cutoff >= 1`, and "culled" as the
    /// negation of the "visible" comparison so a NaN would cull here as it does there).
    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn cone_culling_is_conservative() {
        use glam::Vec3;
        let mesh = crate::procedural::asteroid(Seed::new(700), 48, 1.0, 0.45);
        let built = MeshletMesh::build(&mesh);
        let mut rng = Seed::new(9).rng();
        let (mut culled_apex, mut culled_sphere) = (0, 0);
        let (mut bad_apex, mut bad_sphere) = (0, 0);
        for _ in 0..64 {
            let dir = Vec3::new(
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
            )
            .normalize_or(Vec3::X);
            let camera = dir * rng.range_f32(1.3, 60.0);
            for m in &built.meshlets {
                if m.cone_cutoff >= 1.0 {
                    continue;
                }
                let apex = Vec3::from(m.cone_apex);
                let axis = Vec3::from(m.cone_axis).normalize();
                let center = Vec3::from(m.center);
                let cull_apex = !((apex - camera).normalize().dot(axis) < m.cone_cutoff);
                let cull_sphere = !((center - camera).dot(axis)
                    < m.cone_cutoff * (center - camera).length() + m.radius);
                if !cull_apex && !cull_sphere {
                    continue;
                }
                culled_apex += usize::from(cull_apex);
                culled_sphere += usize::from(cull_sphere);
                let mut front = 0;
                for t in 0..m.triangle_count as usize {
                    let corner = |k: usize| {
                        let local = built.meshlet_triangles[m.triangle_offset as usize + t * 3 + k]
                            as usize;
                        Vec3::from(
                            built.vertices
                                [built.meshlet_vertices[m.vertex_offset as usize + local] as usize]
                                .position,
                        )
                    };
                    let (a, b, c) = (corner(0), corner(1), corner(2));
                    if (b - a).cross(c - a).dot(camera - a) > 0.0 {
                        front += 1;
                    }
                }
                bad_apex += usize::from(cull_apex && front > 0);
                bad_sphere += usize::from(cull_sphere && front > 0);
            }
        }
        eprintln!(
            "apex test: {culled_apex} culled, {bad_apex} wrongly; sphere test: {culled_sphere} culled, {bad_sphere} wrongly"
        );
        assert_eq!(bad_sphere, 0);
        assert_eq!(bad_apex, 0);
    }

    /// The same check through an instance transform, exactly as the task shader does it in
    /// world space (uniform scale, rotation, translation, `f32` throughout).
    #[test]
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn cone_culling_is_conservative_in_world_space() {
        use glam::{Mat4, Quat, Vec3, Vec4};
        let mesh = crate::procedural::asteroid(Seed::new(700), 48, 1.0, 0.45);
        let built = MeshletMesh::build(&mesh);
        let mut rng = Seed::new(11).rng();
        let (mut culled, mut bad) = (0, 0);
        for _ in 0..48 {
            let rotation = Quat::from_euler(
                glam::EulerRot::XYZ,
                rng.range_f32(0.0, std::f32::consts::TAU),
                rng.range_f32(0.0, std::f32::consts::TAU),
                rng.range_f32(0.0, std::f32::consts::TAU),
            );
            let scale = rng.range_f32(0.6, 1.5);
            let translation = Vec3::new(
                rng.range_f32(-600.0, 600.0),
                rng.range_f32(-100.0, 100.0),
                rng.range_f32(-300.0, 300.0),
            );
            let model =
                Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, translation);
            let dir = Vec3::new(
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
                rng.next_f32() * 2.0 - 1.0,
            )
            .normalize_or(Vec3::X);
            let camera = translation + dir * rng.range_f32(3.0, 80.0);
            for m in &built.meshlets {
                if m.cone_cutoff >= 1.0 {
                    continue;
                }
                let apex = model.transform_point3(Vec3::from(m.cone_apex));
                let axis = (model * Vec4::from((Vec3::from(m.cone_axis), 0.0)))
                    .truncate()
                    .normalize();
                // Written as the shader writes it: "visible" is the comparison, culled is its negation.
                let cull = !((apex - camera).normalize().dot(axis) < m.cone_cutoff);
                if !cull {
                    continue;
                }
                culled += 1;
                let mut front = 0;
                for t in 0..m.triangle_count as usize {
                    let corner = |k: usize| {
                        let local = built.meshlet_triangles[m.triangle_offset as usize + t * 3 + k]
                            as usize;
                        model.transform_point3(Vec3::from(
                            built.vertices
                                [built.meshlet_vertices[m.vertex_offset as usize + local] as usize]
                                .position,
                        ))
                    };
                    let (a, b, c) = (corner(0), corner(1), corner(2));
                    if (b - a).cross(c - a).dot(camera - a) > 0.0 {
                        front += 1;
                    }
                }
                bad += usize::from(front > 0);
            }
        }
        eprintln!("world space: {culled} culled, {bad} wrongly");
        assert_eq!(bad, 0);
    }
}

//! Meshlet building with meshoptimizer.
//!
//! A meshlet is a cluster of at most 64 vertices and 124 triangles with a bounding sphere, a
//! normal cone and its place in the cluster LOD DAG (`lod.rs`), the unit of work of the
//! task/mesh pipeline (`shaders/meshlet.slang`). The sizes follow NVIDIA's mesh-shader
//! guidance and the limits reported by the device.

use bytemuck::{Pod, Zeroable};

use crate::lod;
use crate::page::{self, PAGE_SIZE, PagedVertex};
use crate::procedural::TriMesh;

/// Maximum vertices per meshlet.
pub const MESHLET_MAX_VERTICES: usize = 64;
/// Maximum triangles per meshlet.
pub const MESHLET_MAX_TRIANGLES: usize = 124;

/// A cooking vertex (32 bytes): what the DAG builder simplifies. The GPU reads the 16-byte
/// [`PagedVertex`] of the cluster pages instead.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct GpuVertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Padding.
    pub pad0: f32,
    /// Object-space normal.
    pub normal: [f32; 3],
    /// The vertex's material section as a number (issue #41): an attribute the simplifier
    /// weighs, so a section dissolves into its neighbour only once that is cheap.
    pub section: f32,
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
    /// The page holding the payload (the mesh's page index; a scene rebases it into its
    /// page table).
    pub page: u32,
    /// Byte offset of the payload in the page: `vertex_count` [`PagedVertex`], then
    /// `triangle_count` triangles of three one-byte local indices (see [`crate::page`]).
    pub payload: u32,
    /// Vertices in this meshlet.
    pub vertex_count: u32,
    /// Triangles in this meshlet.
    pub triangle_count: u32,
    /// The page of this cluster's children, the members of the group that produced it
    /// ([`page::PAGE_NONE`] at level 0): the cut refines the cluster only when that page is
    /// resident.
    pub child_page: u32,
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
    /// The material sections of the cluster's triangles (issue #41), `a | b << 8 | split << 16`:
    /// triangles before `split` are in section `a`, the others in `b` (0 for a mesh without
    /// sections). An instance draws section `s` with the row after its own by `s`.
    pub section: u32,
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

/// A mesh cut into clusters at every LOD level and packed into pages, ready for upload.
pub struct MeshletMesh {
    /// Meshlet records, all levels (the hierarchy, always resident).
    pub meshlets: Vec<GpuMeshlet>,
    /// The clusters' payloads, [`PAGE_SIZE`] bytes per page, the roots' pages first; empty
    /// when the pages stay in a file ([`MeshletMesh::page_file`]).
    pub pages: Vec<u8>,
    /// How many pages there are.
    pub page_count: u32,
    /// How many of the first pages hold the roots.
    pub root_pages: u32,
    /// Where the pages lie on disk when they are not in memory (a streamed mesh, see
    /// `crate::cache::load_hierarchy`).
    pub page_file: Option<PageFile>,
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
        let (vertices, indices, vertex_section) = split_sections(mesh);
        let indices = meshopt::optimize_vertex_cache(&indices, vertices.len());
        let mut dag = lod::build_dag(&indices, &vertices, &vertex_section, options.normal_weight);
        let pages = page::pack(&mut dag, &vertices);
        let (center, radius) = bounding_sphere(&mesh.positions);
        Self {
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
        }
    }

    /// Number of LOD levels.
    pub fn levels(&self) -> usize {
        self.clusters_per_level.len()
    }

    /// Vertex `i` of cluster `m`, read from its page (the pages must be in memory).
    pub fn vertex(&self, m: &GpuMeshlet, i: usize) -> PagedVertex {
        let at = m.page as usize * PAGE_SIZE + m.payload as usize + i * size_of::<PagedVertex>();
        bytemuck::pod_read_unaligned(&self.pages[at..at + size_of::<PagedVertex>()])
    }

    /// The three local vertex indices of triangle `t` of cluster `m` (pages in memory).
    pub fn triangle(&self, m: &GpuMeshlet, t: usize) -> [u8; 3] {
        let at = m.page as usize * PAGE_SIZE
            + m.payload as usize
            + m.vertex_count as usize * size_of::<PagedVertex>()
            + t * 3;
        [self.pages[at], self.pages[at + 1], self.pages[at + 2]]
    }

    /// The position of corner `k` of triangle `t` of cluster `m` (pages in memory).
    pub fn corner(&self, m: &GpuMeshlet, t: usize, k: usize) -> [f32; 3] {
        self.vertex(m, self.triangle(m, t)[k] as usize).position
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

/// Where a mesh's pages lie in a file: page `p` is the [`PAGE_SIZE`] bytes at
/// `offset + p × PAGE_SIZE` of `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageFile {
    /// The file (a mesh cache file, `crate::cache`).
    pub path: std::path::PathBuf,
    /// Where page 0 starts.
    pub offset: u64,
}

/// The cooking vertices, the triangle list and each vertex's section. A vertex whose
/// triangles lie in several sections is split into one copy per section (same position and
/// normal): no edge then joins two sections, meshoptimizer treats the copies as a seam and
/// keeps the border while it simplifies, and every triangle the DAG makes lies in the
/// section of its vertices.
fn split_sections(mesh: &TriMesh) -> (Vec<GpuVertex>, Vec<u32>, Vec<u8>) {
    let mut vertices: Vec<GpuVertex> = mesh
        .positions
        .iter()
        .zip(&mesh.normals)
        .map(|(p, n)| GpuVertex {
            position: *p,
            pad0: 0.0,
            normal: *n,
            section: 0.0,
        })
        .collect();
    let mut vertex_section = vec![u8::MAX; vertices.len()];
    let mut copies: std::collections::HashMap<(u32, u8), u32> = std::collections::HashMap::new();
    let mut indices = mesh.indices.clone();
    for (t, tri) in indices.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        let section = mesh.section(t);
        for v in tri {
            let owner = &mut vertex_section[*v as usize];
            if *owner == u8::MAX {
                *owner = section;
            } else if *owner != section {
                let original = *v;
                *v = *copies.entry((original, section)).or_insert_with(|| {
                    vertices.push(GpuVertex {
                        section: f32::from(section),
                        ..vertices[original as usize]
                    });
                    vertex_section.push(section);
                    (vertices.len() - 1) as u32
                });
            }
        }
    }
    // Vertices no triangle uses keep section 0.
    for (v, s) in vertex_section.iter_mut().enumerate() {
        if *s == u8::MAX {
            *s = 0;
        }
        vertices[v].section = f32::from(*s);
    }
    (vertices, indices, vertex_section)
}

/// The section of triangle `t` of a cluster whose packed sections are `packed`
/// ([`GpuMeshlet::section`]).
pub fn triangle_section(packed: u32, t: u32) -> u32 {
    if t < (packed >> 16) & 0xFF {
        packed & 0xFF
    } else {
        (packed >> 8) & 0xFF
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
            assert!(m.page < built.page_count);
            assert_eq!(m.payload as usize % page::PAYLOAD_ALIGN, 0);
            let end = m.payload as usize + page::payload_bytes(m.vertex_count, m.triangle_count);
            assert!(end <= PAGE_SIZE);
            for t in 0..m.triangle_count as usize {
                for local in built.triangle(m, t) {
                    assert!(u32::from(local) < m.vertex_count);
                }
            }
        }
        assert_eq!(built.pages.len() % PAGE_SIZE, 0);
    }

    /// Every cluster's payload lies in its page without overlapping another's; the roots
    /// fill the first pages; a group's members share one page, and the clusters it produced
    /// point at it; level 0 points nowhere.
    #[test]
    fn pages_keep_groups_whole_and_point_at_the_children() {
        let mesh = crate::procedural::asteroid(Seed::new(700), 64, 1.0, 0.45);
        let built = MeshletMesh::build(&mesh);
        assert!(built.page_count >= 2, "pages {}", built.page_count);
        assert_eq!(built.pages.len(), built.page_count as usize * PAGE_SIZE);
        assert!(built.root_pages >= 1);
        let mut spans: Vec<(usize, usize)> = built
            .meshlets
            .iter()
            .map(|m| {
                let start = m.page as usize * PAGE_SIZE + m.payload as usize;
                (
                    start,
                    start + page::payload_bytes(m.vertex_count, m.triangle_count),
                )
            })
            .collect();
        spans.sort_unstable();
        for pair in spans.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "payloads overlap: {pair:?}");
        }
        for m in &built.meshlets {
            let root = m.parent_error.is_infinite();
            assert_eq!(
                root,
                m.page < built.root_pages,
                "roots and only roots in root pages"
            );
            if m.lod_level == 0 {
                assert_eq!(m.child_page, page::PAGE_NONE);
            } else {
                assert!(m.child_page >= built.root_pages && m.child_page < built.page_count);
            }
        }
        // The children of a cluster are the clusters whose parent is its group: they share
        // its `self` values as their `parent` ones and all lie in its `child_page`.
        let key = |c: [f32; 3], r: f32, e: f32| (c.map(f32::to_bits), r.to_bits(), e.to_bits());
        let mut child_page = std::collections::HashMap::new();
        for m in built.meshlets.iter().filter(|m| m.lod_level > 0) {
            let k = key(m.self_center, m.self_radius, m.self_error);
            assert_eq!(*child_page.entry(k).or_insert(m.child_page), m.child_page);
        }
        let mut children = 0;
        for m in built.meshlets.iter().filter(|m| m.parent_error.is_finite()) {
            let k = key(m.parent_center, m.parent_radius, m.parent_error);
            assert_eq!(
                child_page.get(&k),
                Some(&m.page),
                "a child outside its page"
            );
            children += 1;
        }
        assert!(children > 0);
    }

    /// The paged vertices are the cooked ones: exact positions, normals within 0.01°.
    #[test]
    fn paged_vertices_keep_the_positions_exactly() {
        let mesh = crate::procedural::asteroid(Seed::new(7), 24, 1.0, 0.2);
        let built = MeshletMesh::build(&mesh);
        let mut matched = 0;
        for m in level0(&built) {
            for i in 0..m.vertex_count as usize {
                let v = built.vertex(m, i);
                let source = mesh
                    .positions
                    .iter()
                    .position(|p| *p == v.position)
                    .expect("a paged position that is not a mesh vertex");
                let n = mesh.normals[source];
                let d = page::decode_normal(v.normal);
                let dot: f32 = (0..3).map(|k| n[k] * d[k]).sum();
                assert!(dot > 0.999_99, "normal off by {dot}");
                matched += 1;
            }
        }
        assert!(matched > 0);
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
                    let corner = |k: usize| Vec3::from(built.corner(m, t, k));
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
                    let corner =
                        |k: usize| model.transform_point3(Vec3::from(built.corner(m, t, k)));
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

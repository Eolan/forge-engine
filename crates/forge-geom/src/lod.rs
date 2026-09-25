//! The cluster LOD DAG (Nanite 2021, meshoptimizer's cluster LOD recipe).
//!
//! Level 0 is the mesh cut into clusters. Each level partitions the clusters of the level
//! below into spatial groups of about [`GROUP_SIZE`], locks the vertices on group borders,
//! simplifies each group's triangles to half, re-clusters the result and records, for every
//! cluster, two spheres and two errors:
//!
//! - `self`: the bounds and error of the group that *produced* the cluster (level 0: error 0),
//! - `parent`: the bounds and error of the group that *consumed* it (a root: infinite error).
//!
//! Errors and spheres are monotonic up the DAG (a group's error is at least its children's,
//! its sphere contains theirs), and all clusters of a group share the group's values, so the
//! GPU test "draw when `project(parent) > threshold >= project(self)`" selects exactly one
//! cut through the DAG with no cracks: siblings decide together, and the children of a
//! group decide with the same numbers their parents used.
//!
//! Vertices are shared across levels (simplification only drops indices), so the vertex
//! buffer is the original one; only the index tables grow (about twice the leaf count).
//! Every cluster also records the group it is a member of and the group that produced it:
//! the page packer (`crate::page`) keeps each group in one page and points a cluster at the
//! page of its children.

use meshopt::{PositionDataAdapter, RadiusDataAdapter, SimplifyOptions, VertexDataAdapter};

use crate::meshlet::{GpuMeshlet, GpuVertex, MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES};

/// Clusters per simplification group.
pub const GROUP_SIZE: usize = 8;
/// Upper bound on DAG levels (a mesh halves per level; 16 levels are a 65 536× reduction), also
/// the size of the per-level tables in the GPU `Mesh` record.
pub const MAX_LEVELS: u32 = 16;
/// Metres of simplification error a triangle pays for crossing from one material section
/// into another (issue #41): a building's glass dissolves into its facade once half a metre
/// of error is allowed, a few pixels per window at the cut's 1 px.
const SECTION_WEIGHT: f32 = 0.5;
/// A group that keeps more than this share of its triangles after simplification has
/// stalled; its clusters become roots.
const STALL_RATIO: f32 = 0.85;

/// The DAG's clusters: their GPU records (without their page placement, see
/// [`crate::page`]), where their vertices and triangles lie in the two index tables, and
/// the groups they belong to.
pub struct ClusterDag {
    /// Every cluster of every level.
    pub meshlets: Vec<GpuMeshlet>,
    /// Per cluster: its window of `meshlet_vertices` and `meshlet_triangles`.
    pub ranges: Vec<ClusterRange>,
    /// Global vertex indices, per cluster.
    pub meshlet_vertices: Vec<u32>,
    /// Local triangle indices (3 bytes each).
    pub meshlet_triangles: Vec<u8>,
    /// Per cluster: the group it is a member of ([`NO_GROUP`] when it was never grouped:
    /// the last cluster, or the level cap).
    pub cluster_group: Vec<u32>,
    /// Per cluster: the group whose simplification produced it ([`NO_GROUP`] at level 0).
    pub cluster_source: Vec<u32>,
    /// Clusters per level.
    pub clusters_per_level: Vec<u32>,
    /// Triangles over all levels.
    pub triangle_count: usize,
}

/// No group (see [`ClusterDag::cluster_group`]).
pub const NO_GROUP: u32 = u32::MAX;

/// Where one cluster's data lies in the DAG's index tables.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClusterRange {
    /// First entry in `meshlet_vertices`.
    pub vertex_offset: u32,
    /// First byte in `meshlet_triangles`.
    pub triangle_offset: u32,
}

struct Record {
    gpu: GpuMeshlet,
    range: ClusterRange,
    group: u32,
    source: u32,
    /// Global vertex indices of the cluster's triangles (needed to merge and partition).
    indices: Vec<u32>,
}

/// Builds the DAG of `indices` over `vertices`. `indices` must be a valid triangle list.
/// `vertex_section` gives every vertex's material section; no triangle may join two sections
/// (`crate::meshlet` splits the vertices on section borders), and every cluster holds one.
/// `normal_weight` adds the vertex normals to the simplification error (metres per unit of
/// normal change; 0: geometry only, see `crate::meshlet::CookOptions`).
pub fn build_dag(
    indices: &[u32],
    vertices: &[GpuVertex],
    vertex_section: &[u8],
    normal_weight: f32,
) -> ClusterDag {
    let mut dag = ClusterDag {
        meshlets: Vec::new(),
        ranges: Vec::new(),
        meshlet_vertices: Vec::new(),
        meshlet_triangles: Vec::new(),
        cluster_group: Vec::new(),
        cluster_source: Vec::new(),
        clusters_per_level: Vec::new(),
        triangle_count: 0,
    };
    let mut records: Vec<Record> = Vec::new();
    let mut compactor = Compactor::new(vertices.len());
    let mut locked = vec![false; vertices.len()];
    // Vertices at one position (the copies on a section border) are one vertex for grouping
    // and locking: a border shared by two groups is locked in both of its copies.
    let remap = meshopt::generate_position_remap(
        &VertexDataAdapter::new(
            bytemuck::cast_slice(vertices),
            std::mem::size_of::<GpuVertex>(),
            0,
        )
        .expect("vertex adapter"),
    );
    let sections = Sections {
        of_vertex: vertex_section,
    };
    // Meshes with sections simplify across their borders (meshoptimizer's permissive mode)
    // at the price of `SECTION_WEIGHT`: while the section borders were seams, the windows'
    // outlines outlived every level and far buildings drew 40 % more clusters.
    let has_sections = vertex_section.iter().any(|&s| s != 0);

    // Level 0.
    let whole = compactor.subset(indices, vertices, &locked);
    let mut current = emit_clusters(
        &mut dag,
        &mut records,
        &whole,
        &whole.indices,
        &sections,
        0,
        ([0.0; 3], 0.0),
        0.0,
    );
    dag.clusters_per_level.push(current.len() as u32);

    let mut level = 1;
    let mut group_id = 0_u32;
    while current.len() > 1 && level < MAX_LEVELS {
        // Partition the level's clusters into spatial groups.
        let cluster_indices: Vec<u32> = current
            .iter()
            .flat_map(|&c| records[c].indices.iter().map(|&v| remap[v as usize]))
            .collect();
        let counts: Vec<u32> = current
            .iter()
            .map(|&c| records[c].indices.len() as u32)
            .collect();
        let mut partition = vec![0_u32; current.len()];
        let group_count = meshopt::partition_clusters(
            &mut partition,
            &cluster_indices,
            &counts,
            vertices.len(),
            GROUP_SIZE,
        );
        let mut groups: Vec<Vec<usize>> = vec![Vec::new(); group_count];
        for (slot, &c) in current.iter().enumerate() {
            groups[partition[slot] as usize].push(c);
        }

        // Lock the vertices shared between groups so neighbouring groups stay watertight.
        let mut vertex_group = vec![u32::MAX; vertices.len()];
        locked.fill(false);
        for (slot, &c) in current.iter().enumerate() {
            let group = partition[slot];
            for &v in &records[c].indices {
                let canonical = remap[v as usize] as usize;
                let owner = &mut vertex_group[canonical];
                if *owner == u32::MAX {
                    *owner = group;
                } else if *owner != group {
                    locked[canonical] = true;
                }
            }
        }
        for (v, &canonical) in remap.iter().enumerate() {
            locked[v] = locked[canonical as usize];
        }

        let mut next = Vec::new();
        let mut progressed = false;
        for members in groups.iter().filter(|m| !m.is_empty()) {
            let group = group_id;
            group_id += 1;
            for &c in members {
                records[c].group = group;
            }
            let merged: Vec<u32> = members
                .iter()
                .flat_map(|&c| records[c].indices.iter().copied())
                .collect();
            let subset = compactor.subset(&merged, vertices, &locked);
            let target = (merged.len() / 3).div_ceil(2).max(1) * 3;
            let mut simplify_error = 0.0_f32;
            let mut options = SimplifyOptions::LockBorder | SimplifyOptions::ErrorAbsolute;
            if has_sections {
                options |= SimplifyOptions::Permissive;
            }
            // The normals are the vertex's floats 4..7 and its section float 7: read in place.
            let floats: &[f32] = bytemuck::cast_slice(&subset.vertices);
            let (attributes, weights): (&[f32], &[f32]) = match (normal_weight > 0.0, has_sections)
            {
                (true, true) => (
                    &floats[4..],
                    &[normal_weight, normal_weight, normal_weight, SECTION_WEIGHT],
                ),
                (true, false) => (&floats[4..], &[normal_weight; 3]),
                (false, true) => (&floats[7..], &[SECTION_WEIGHT]),
                (false, false) => (&[], &[]),
            };
            let simplified = if !weights.is_empty() {
                meshopt::simplify_with_attributes_and_locks(
                    &subset.indices,
                    &subset.adapter(),
                    attributes,
                    weights,
                    std::mem::size_of::<GpuVertex>(),
                    &subset.locked,
                    target,
                    f32::MAX,
                    options,
                    Some(&mut simplify_error),
                )
            } else {
                meshopt::simplify_with_locks(
                    &subset.indices,
                    &subset.adapter(),
                    &subset.locked,
                    target,
                    f32::MAX,
                    options,
                    Some(&mut simplify_error),
                )
            };
            if simplified.is_empty() || simplified.len() as f32 > merged.len() as f32 * STALL_RATIO
            {
                // Could not simplify: these clusters are roots of their branch.
                continue;
            }
            progressed = true;
            // The group's bounds and error: monotonic over the children.
            let children_spheres: Vec<[f32; 4]> = members
                .iter()
                .map(|&c| {
                    [
                        records[c].gpu.self_center[0],
                        records[c].gpu.self_center[1],
                        records[c].gpu.self_center[2],
                        records[c].gpu.self_radius,
                    ]
                })
                .collect();
            let sphere = enclosing_sphere(&children_spheres);
            let error = members
                .iter()
                .map(|&c| records[c].gpu.self_error)
                .fold(simplify_error, f32::max);
            for &c in members {
                records[c].gpu.parent_center = sphere.0;
                records[c].gpu.parent_radius = sphere.1;
                records[c].gpu.parent_error = error;
            }
            let produced = emit_clusters(
                &mut dag,
                &mut records,
                &subset,
                &simplified,
                &sections,
                level,
                sphere,
                error,
            );
            for &c in &produced {
                records[c].source = group;
            }
            next.extend(produced);
        }
        if !progressed {
            break;
        }
        dag.clusters_per_level.push(next.len() as u32);
        current = next;
        level += 1;
    }
    // Everything without a parent is a root (infinite parent error: never "too coarse").
    for record in &mut records {
        if record.gpu.parent_error == 0.0 && record.gpu.parent_radius == 0.0 {
            record.gpu.parent_error = f32::INFINITY;
            record.gpu.parent_center = record.gpu.self_center;
            record.gpu.parent_radius = record.gpu.self_radius;
        }
    }
    for record in records {
        dag.meshlets.push(record.gpu);
        dag.ranges.push(record.range);
        dag.cluster_group.push(record.group);
        dag.cluster_source.push(record.source);
    }
    while !dag.meshlet_triangles.len().is_multiple_of(4) {
        dag.meshlet_triangles.push(0);
    }
    dag
}

/// Part of the mesh (the whole mesh, or a group's triangles) over a compact copy of the
/// vertices it uses. meshoptimizer's simplifier and clusteriser size their per-vertex tables
/// by the vertex buffer they are given: handed the whole mesh for every group, the cost of a
/// level was groups × vertices, and a 3 M-triangle mesh took 8.5 minutes to cook.
struct Subset {
    /// The used vertices, in the mesh's order.
    vertices: Vec<GpuVertex>,
    /// Their lock flags (shared with another group).
    locked: Vec<bool>,
    /// The mesh's index of each local vertex.
    global: Vec<u32>,
    /// The triangles, in local indices.
    indices: Vec<u32>,
}

impl Subset {
    fn adapter(&self) -> VertexDataAdapter<'_> {
        VertexDataAdapter::new(
            bytemuck::cast_slice(&self.vertices),
            std::mem::size_of::<GpuVertex>(),
            0,
        )
        .expect("vertex adapter")
    }
}

/// Builds [`Subset`]s with one mesh-sized table, reset after each subset.
struct Compactor {
    /// Local index of each mesh vertex in the subset being built, `u32::MAX` elsewhere.
    local: Vec<u32>,
}

impl Compactor {
    fn new(vertex_count: usize) -> Self {
        Self {
            local: vec![u32::MAX; vertex_count],
        }
    }

    /// The subset of `indices`, its vertices numbered in the mesh's order (so meshoptimizer
    /// breaks ties between vertices as it would on the whole mesh).
    fn subset(&mut self, indices: &[u32], vertices: &[GpuVertex], locked: &[bool]) -> Subset {
        let mut global = Vec::new();
        for &g in indices {
            let local = &mut self.local[g as usize];
            if *local == u32::MAX {
                *local = 0;
                global.push(g);
            }
        }
        global.sort_unstable();
        for (i, &g) in global.iter().enumerate() {
            self.local[g as usize] = i as u32;
        }
        let subset = Subset {
            vertices: global.iter().map(|&g| vertices[g as usize]).collect(),
            locked: global.iter().map(|&g| locked[g as usize]).collect(),
            indices: indices.iter().map(|&g| self.local[g as usize]).collect(),
            global,
        };
        for &g in &subset.global {
            self.local[g as usize] = u32::MAX;
        }
        subset
    }
}

/// Cuts `indices` (local to `subset`) into clusters, appends them to the tables and the
/// records in mesh indices, and returns the new records' ids. `self_sphere` / `self_error`
/// are the producing group's values.
#[allow(clippy::too_many_arguments)]
fn emit_clusters(
    dag: &mut ClusterDag,
    records: &mut Vec<Record>,
    subset: &Subset,
    indices: &[u32],
    sections: &Sections<'_>,
    level: u32,
    self_sphere: ([f32; 3], f32),
    self_error: f32,
) -> Vec<usize> {
    let adapter = subset.adapter();
    let section_of = |local: u32| sections.of_vertex[subset.global[local as usize] as usize];
    let build = |indices: &[u32]| {
        meshopt::build_meshlets(
            indices,
            &adapter,
            MESHLET_MAX_VERTICES,
            MESHLET_MAX_TRIANGLES,
            0.5,
        )
    };
    let mut ids = Vec::new();
    let mut emit = |vertices: &[u32], triangles: &[u8], dag: &mut ClusterDag| {
        // A cluster holds at most two sections, its triangles sorted by section: the
        // record packs both and where the second starts (`GpuMeshlet::section`).
        let section = |t: &[u8; 3]| section_of(vertices[t[0] as usize]);
        let mut order: Vec<&[u8; 3]> = triangles.as_chunks::<3>().0.iter().collect();
        order.sort_by_key(|t| section(t));
        let first = section(order[0]);
        let last = section(order[order.len() - 1]);
        let split = order.iter().take_while(|t| section(t) == first).count();
        let packed = if first == last {
            u32::from(first) | u32::from(first) << 8
        } else {
            u32::from(first) | u32::from(last) << 8 | (split as u32) << 16
        };
        let sorted: Vec<u8> = order.iter().flat_map(|t| t.iter().copied()).collect();
        let meshlet = meshopt::Meshlet {
            vertices,
            triangles: &sorted,
        };
        let bounds = meshopt::compute_meshlet_bounds(meshlet, &adapter);
        let vertex_offset = dag.meshlet_vertices.len() as u32;
        let triangle_offset = dag.meshlet_triangles.len() as u32;
        dag.meshlet_vertices
            .extend(vertices.iter().map(|&v| subset.global[v as usize]));
        dag.meshlet_triangles.extend_from_slice(&sorted);
        let global: Vec<u32> = sorted
            .iter()
            .map(|&local| subset.global[vertices[local as usize] as usize])
            .collect();
        let triangle_count = (sorted.len() / 3) as u32;
        dag.triangle_count += triangle_count as usize;
        // Level 0 clusters use their own sphere as `self` (error 0, always fine enough).
        let (self_center, self_radius) = if level == 0 {
            (bounds.center, bounds.radius)
        } else {
            self_sphere
        };
        let gpu = GpuMeshlet {
            center: bounds.center,
            radius: bounds.radius,
            cone_apex: bounds.cone_apex,
            cone_cutoff: bounds.cone_cutoff,
            cone_axis: bounds.cone_axis,
            page: 0,
            payload: 0,
            vertex_count: vertices.len() as u32,
            triangle_count,
            child_page: crate::page::PAGE_NONE,
            self_center,
            self_radius,
            parent_center: [0.0; 3],
            parent_radius: 0.0,
            self_error,
            parent_error: 0.0,
            lod_level: level,
            section: packed,
        };
        ids.push(records.len());
        records.push(Record {
            gpu,
            range: ClusterRange {
                vertex_offset,
                triangle_offset,
            },
            group: NO_GROUP,
            source: NO_GROUP,
            indices: global,
        });
    };
    let built = build(indices);
    for meshlet in built.iter() {
        let mut present: Vec<u8> = meshlet
            .triangles
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| section_of(meshlet.vertices[t[0] as usize]))
            .collect();
        present.sort_unstable();
        present.dedup();
        if present.len() <= 2 {
            emit(meshlet.vertices, meshlet.triangles, dag);
            continue;
        }
        // More than two sections met in one cluster: cluster its triangles again, section by
        // section.
        for &s in &present {
            let part: Vec<u32> = meshlet
                .triangles
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|t| section_of(meshlet.vertices[t[0] as usize]) == s)
                .flat_map(|t| t.iter().map(|&l| meshlet.vertices[l as usize]))
                .collect();
            for piece in build(&part).iter() {
                emit(piece.vertices, piece.triangles, dag);
            }
        }
    }
    ids
}

/// Every vertex's material section (see [`build_dag`]).
struct Sections<'a> {
    of_vertex: &'a [u8],
}

/// A sphere containing every sphere of `spheres` (`[x, y, z, r]`).
fn enclosing_sphere(spheres: &[[f32; 4]]) -> ([f32; 3], f32) {
    if spheres.is_empty() {
        return ([0.0; 3], 0.0);
    }
    let bytes: &[u8] = bytemuck::cast_slice(spheres);
    let stride = std::mem::size_of::<[f32; 4]>();
    let positions = PositionDataAdapter {
        data: bytes,
        position_count: spheres.len(),
        position_stride: stride,
        position_offset: 0,
    };
    let radii = RadiusDataAdapter {
        data: bytes,
        radius_count: spheres.len(),
        radius_stride: stride,
        radius_offset: 12,
    };
    let sphere = meshopt::compute_sphere_bounds(positions, Some(radii));
    // Guard against a too-tight fit from the fast fitter: grow to contain every child exactly.
    let mut radius = sphere.radius;
    for s in spheres {
        let d = ((s[0] - sphere.center[0]).powi(2)
            + (s[1] - sphere.center[1]).powi(2)
            + (s[2] - sphere.center[2]).powi(2))
        .sqrt();
        radius = radius.max(d + s[3]);
    }
    (sphere.center, radius)
}

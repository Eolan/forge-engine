//! Cluster pages (issue #36): the unit geometry is stored, streamed and made resident in
//! (D-018, D-025).
//!
//! A page is [`PAGE_SIZE`] bytes of cluster payloads packed back to back. A cluster's
//! payload lies at a 16-byte offset of its page (`GpuMeshlet::payload`): its vertices
//! ([`PagedVertex`], 16 bytes each), then its triangles (three one-byte local indices each),
//! padded to 16 bytes. A mesh with texture coordinates (D-047) adds its UV stream after them
//! ([`uv_offset`]): the cluster's UV range (its minimum and extent, four `f32`), then each
//! vertex's UV as two 16-bit unorms within that range, padded to 16 bytes. Every cluster carries its own copy of its vertices, so a page needs
//! nothing outside itself.
//!
//! Packing keeps together what the LOD cut decides together:
//! - **Roots first,** in the first [`Pages::root_pages`] pages. They stay resident, so the
//!   coarsest picture of every mesh is always there to fall back on.
//! - **A group never spans two pages.** A DAG group's members are the children of the
//!   clusters its simplification produced. The cut refines those clusters only when all
//!   their children are resident, and one page decides that: every cluster records the
//!   page of its children as its `child_page` ([`PAGE_NONE`] at level 0).
//! - **Groups go level by level,** finest first, in Morton order of their spheres within a
//!   level, so a page holds neighbours of one level.

use bytemuck::{Pod, Zeroable};

use crate::lod::{ClusterDag, NO_GROUP};
use crate::meshlet::GpuVertex;

/// Bytes per page (D-018: fixed 128 KB pages, after Nanite).
pub const PAGE_SIZE: usize = 128 * 1024;
/// No page: the `child_page` of a level-0 cluster.
pub const PAGE_NONE: u32 = u32::MAX;
/// Payloads start at multiples of this, so a page is an array of [`PagedVertex`].
pub const PAYLOAD_ALIGN: usize = 16;

/// A vertex in a page (16 bytes): the object-space position, exact, and the normal in
/// 16-bit octahedral form ([`encode_normal`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PagedVertex {
    /// Object-space position.
    pub position: [f32; 3],
    /// Octahedral normal: x in the low 16 bits, y in the high, signed normalised.
    pub normal: u32,
}

/// Octahedral encoding of a unit normal (Meyer et al. 2010, "On floating-point normal
/// vectors"), each component as a 16-bit signed normalised integer: at most 0.0035° off.
/// A zero vector encodes as +Z.
pub fn encode_normal(n: [f32; 3]) -> u32 {
    let l1 = n[0].abs() + n[1].abs() + n[2].abs();
    if l1 == 0.0 || !l1.is_finite() {
        return 0;
    }
    let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
    let (mut x, mut y) = (n[0] / l1, n[1] / l1);
    if n[2] < 0.0 {
        let (ox, oy) = (x, y);
        x = (1.0 - oy.abs()) * sign(ox);
        y = (1.0 - ox.abs()) * sign(oy);
    }
    let q = |v: f32| u32::from((v.clamp(-1.0, 1.0) * 32767.0).round() as i16 as u16);
    q(x) | (q(y) << 16)
}

/// The unit normal [`encode_normal`] packed (`decode_normal` in `meshlet.slang` does the
/// same on the GPU).
pub fn decode_normal(e: u32) -> [f32; 3] {
    let x = f32::from(e as u16 as i16) / 32767.0;
    let y = f32::from((e >> 16) as u16 as i16) / 32767.0;
    let (x, y) = (x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0));
    let z = 1.0 - x.abs() - y.abs();
    let t = (-z).max(0.0);
    let x = if x >= 0.0 { x - t } else { x + t };
    let y = if y >= 0.0 { y - t } else { y + t };
    let l = (x * x + y * y + z * z).sqrt();
    [x / l, y / l, z / l]
}

/// A mesh's pages.
#[derive(Clone, Debug, Default)]
pub struct Pages {
    /// `PAGE_SIZE` bytes per page.
    pub bytes: Vec<u8>,
    /// The first pages, holding the roots (always resident).
    pub root_pages: u32,
}

impl Pages {
    /// How many pages there are.
    pub fn count(&self) -> u32 {
        (self.bytes.len() / PAGE_SIZE) as u32
    }
}

/// Bytes of a cluster's payload with `vertices` and `triangles`, and its UV stream when `uvs`.
pub fn payload_bytes(vertices: u32, triangles: u32, uvs: bool) -> usize {
    let stream = if uvs {
        UV_RANGE_BYTES + (vertices as usize * 4).next_multiple_of(PAYLOAD_ALIGN)
    } else {
        0
    };
    uv_offset(vertices, triangles) + stream
}

/// Where a cluster's UV stream starts in its payload (D-047): after its vertices and its
/// triangles.
pub fn uv_offset(vertices: u32, triangles: u32) -> usize {
    (vertices as usize * size_of::<PagedVertex>() + triangles as usize * 3)
        .next_multiple_of(PAYLOAD_ALIGN)
}

/// Bytes of a UV stream's range: the cluster's UV minimum and extent, two `f32` each.
pub const UV_RANGE_BYTES: usize = 16;

/// The UV range of a cluster whose vertices have `uvs`: their minimum and extent.
pub fn uv_range(uvs: impl IntoIterator<Item = [f32; 2]>) -> [f32; 4] {
    let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
    for uv in uvs {
        for i in 0..2 {
            min[i] = min[i].min(uv[i]);
            max[i] = max[i].max(uv[i]);
        }
    }
    if min[0] > max[0] {
        return [0.0; 4];
    }
    [min[0], min[1], max[0] - min[0], max[1] - min[1]]
}

/// `uv` as two 16-bit unorms within `range` (x in the low 16 bits, y in the high).
pub fn encode_uv(uv: [f32; 2], range: [f32; 4]) -> u32 {
    let q = |i: usize| {
        let extent = range[2 + i];
        if extent > 0.0 {
            (((uv[i] - range[i]) / extent).clamp(0.0, 1.0) * 65535.0).round() as u32
        } else {
            0
        }
    };
    q(0) | (q(1) << 16)
}

/// The UV [`encode_uv`] packed within `range`.
pub fn decode_uv(e: u32, range: [f32; 4]) -> [f32; 2] {
    [
        range[0] + (e & 0xFFFF) as f32 / 65535.0 * range[2],
        range[1] + (e >> 16) as f32 / 65535.0 * range[3],
    ]
}

/// Appends payloads to pages, opening a new page when one does not fit.
struct Packer {
    bytes: Vec<u8>,
    /// Bytes used in the last page (`PAGE_SIZE` when a new one must open).
    cursor: usize,
}

impl Packer {
    /// Where `size` bytes go: a new page when they do not fit in the current one.
    fn reserve(&mut self, size: usize) -> (u32, usize) {
        assert!(size <= PAGE_SIZE, "a group of {size} bytes exceeds a page");
        if self.cursor + size > PAGE_SIZE {
            self.bytes.resize(self.bytes.len() + PAGE_SIZE, 0);
            self.cursor = 0;
        }
        let page = (self.bytes.len() / PAGE_SIZE - 1) as u32;
        let offset = self.cursor;
        self.cursor += size;
        (page, offset)
    }

    fn page_count(&self) -> u32 {
        (self.bytes.len() / PAGE_SIZE) as u32
    }
}

/// Writes cluster `c`'s payload at `page`/`offset` and records the place in its record.
fn write_payload(
    dag: &mut ClusterDag,
    vertices: &[GpuVertex],
    uvs: bool,
    bytes: &mut [u8],
    c: usize,
    page: u32,
    offset: usize,
) {
    let m = dag.meshlets[c];
    let range = dag.ranges[c];
    let base = page as usize * PAGE_SIZE + offset;
    let mut at = base;
    for i in 0..m.vertex_count as usize {
        let v = vertices[dag.meshlet_vertices[range.vertex_offset as usize + i] as usize];
        let paged = PagedVertex {
            position: v.position,
            normal: encode_normal(v.normal),
        };
        bytes[at..at + size_of::<PagedVertex>()].copy_from_slice(bytemuck::bytes_of(&paged));
        at += size_of::<PagedVertex>();
    }
    let t = range.triangle_offset as usize;
    let n = m.triangle_count as usize * 3;
    bytes[at..at + n].copy_from_slice(&dag.meshlet_triangles[t..t + n]);
    if uvs {
        let local =
            |i: usize| vertices[dag.meshlet_vertices[range.vertex_offset as usize + i] as usize];
        let count = m.vertex_count as usize;
        let uv_range = uv_range((0..count).map(|i| local(i).uv));
        let mut at = base + uv_offset(m.vertex_count, m.triangle_count);
        bytes[at..at + UV_RANGE_BYTES].copy_from_slice(bytemuck::bytes_of(&uv_range));
        at += UV_RANGE_BYTES;
        for i in 0..count {
            let e = encode_uv(local(i).uv, uv_range);
            bytes[at..at + 4].copy_from_slice(&e.to_le_bytes());
            at += 4;
        }
    }
    dag.meshlets[c].page = page;
    dag.meshlets[c].payload = offset as u32;
}

/// A 30-bit Morton code of `p` within `min`..`max`.
fn morton(p: [f32; 3], min: [f32; 3], max: [f32; 3]) -> u32 {
    let spread = |v: u32| {
        let mut x = v & 0x3ff;
        x = (x | (x << 16)) & 0x0300_00ff;
        x = (x | (x << 8)) & 0x0300_f00f;
        x = (x | (x << 4)) & 0x030c_30c3;
        (x | (x << 2)) & 0x0924_9249
    };
    let q = |i: usize| {
        let extent = (max[i] - min[i]).max(f32::MIN_POSITIVE);
        (((p[i] - min[i]) / extent).clamp(0.0, 1.0) * 1023.0) as u32
    };
    spread(q(0)) | (spread(q(1)) << 1) | (spread(q(2)) << 2)
}

/// Packs the DAG's clusters into pages (see the module notes) and fills every record's
/// `page`, `payload` and `child_page`. With `uvs` every payload carries its UV stream.
pub fn pack(dag: &mut ClusterDag, vertices: &[GpuVertex], uvs: bool) -> Pages {
    let n = dag.meshlets.len();
    let size = |dag: &ClusterDag, c: usize| {
        payload_bytes(
            dag.meshlets[c].vertex_count,
            dag.meshlets[c].triangle_count,
            uvs,
        )
    };
    let mut packer = Packer {
        bytes: Vec::new(),
        cursor: PAGE_SIZE,
    };

    // The roots, cluster by cluster.
    for c in 0..n {
        if dag.meshlets[c].parent_error.is_infinite() {
            let (page, offset) = packer.reserve(size(dag, c));
            write_payload(dag, vertices, uvs, &mut packer.bytes, c, page, offset);
        }
    }
    let root_pages = packer.page_count();

    // Every other cluster is a member of a group that was simplified.
    let group_count = dag
        .cluster_group
        .iter()
        .filter(|&&g| g != NO_GROUP)
        .map(|&g| g as usize + 1)
        .max()
        .unwrap_or(0);
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); group_count];
    for c in 0..n {
        if !dag.meshlets[c].parent_error.is_infinite() {
            let g = dag.cluster_group[c];
            assert_ne!(g, NO_GROUP, "cluster {c} has a parent but no group");
            members[g as usize].push(c);
        }
    }
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for m in &dag.meshlets {
        for i in 0..3 {
            min[i] = min[i].min(m.center[i]);
            max[i] = max[i].max(m.center[i]);
        }
    }
    let mut order: Vec<(u32, u32, usize)> = members
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.is_empty())
        .map(|(g, m)| {
            let first = &dag.meshlets[m[0]];
            (first.lod_level, morton(first.parent_center, min, max), g)
        })
        .collect();
    order.sort_unstable();

    packer.cursor = PAGE_SIZE; // groups start on a page of their own
    let mut group_page = vec![PAGE_NONE; group_count];
    for &(_, _, g) in &order {
        let total: usize = members[g].iter().map(|&c| size(dag, c)).sum();
        let (page, mut offset) = packer.reserve(total);
        for &c in &members[g] {
            write_payload(dag, vertices, uvs, &mut packer.bytes, c, page, offset);
            offset += size(dag, c);
        }
        group_page[g] = page;
    }
    for c in 0..n {
        let source = dag.cluster_source[c];
        dag.meshlets[c].child_page = if source == NO_GROUP {
            PAGE_NONE
        } else {
            let page = group_page[source as usize];
            assert_ne!(page, PAGE_NONE, "cluster {c} comes from an unpacked group");
            page
        };
    }
    Pages {
        bytes: packer.bytes,
        root_pages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normals_survive_the_octahedral_encoding() {
        let mut worst = 0.0_f64;
        // The angle between two directions, robust near zero (acos of a dot is not).
        let angle = |a: [f32; 3], b: [f32; 3]| -> f64 {
            let (a, b) = (a.map(f64::from), b.map(f64::from));
            let cross = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let sin = cross.iter().map(|c| c * c).sum::<f64>().sqrt();
            let cos: f64 = (0..3).map(|i| a[i] * b[i]).sum();
            sin.atan2(cos).to_degrees()
        };
        for i in 0..2000 {
            // A spiral over the sphere, both hemispheres, plus the axes.
            let t = i as f32 / 1999.0;
            let z = 1.0 - 2.0 * t;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let a = i as f32 * 2.399_963;
            let n = [r * a.cos(), r * a.sin(), z];
            let d = decode_normal(encode_normal(n));
            worst = worst.max(angle(n, d));
        }
        for n in [
            [1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ] {
            let d = decode_normal(encode_normal(n));
            worst = worst.max(angle(n, d));
        }
        // Within 0.01°: far below a shading difference.
        eprintln!("worst normal error {worst}°");
        assert!(worst < 0.01, "worst error {worst}°");
        assert_eq!(decode_normal(encode_normal([0.0; 3])), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn morton_codes_follow_each_axis() {
        let (min, max) = ([0.0; 3], [1.0; 3]);
        assert_eq!(morton([0.0; 3], min, max), 0);
        assert!(morton([1.0, 0.0, 0.0], min, max) > morton([0.5, 0.0, 0.0], min, max));
        assert_eq!(morton([1.0; 3], min, max), (1 << 30) - 1);
    }
}

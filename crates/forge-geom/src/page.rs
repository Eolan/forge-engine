//! Cluster pages (issue #36): the unit geometry is stored, streamed and made resident in
//! (D-018, D-025).
//!
//! A page is [`PAGE_SIZE`] bytes of cluster payloads packed back to back. A cluster's
//! payload lies at a 16-byte offset of its page (`GpuMeshlet::payload`), laid out as
//! [`Layout`] says:
//! - **Its vertices, packed** (#218, D-055, [`SECTION_PACKED`] in the cluster's record):
//!   - a 16-byte header: the cluster's origin on its mesh's grid (three `i32`), then its widths
//!     and the grid's exponent ([`Packed`]);
//!   - a 32-bit octahedral normal per vertex;
//!   - each position's offset from the origin, bit-packed at those widths.
//!
//!   The mesh's positions are snapped to the grid before its DAG is built ([`snap`]), so a
//!   vertex two clusters share decodes to the same bits, and a cut has no crack.
//! - **Or its vertices as records** ([`PagedVertex`], 16 bytes each): the skinned meshes',
//!   which the skin pass writes every frame.
//! - **Then its triangles** (three one-byte local indices each), padded to 16 bytes.
//! - **A mesh with texture coordinates** (D-047) adds its UV stream after them: the cluster's
//!   UV range (its minimum and extent, four `f32`), then each vertex's UV as two 16-bit unorms
//!   within that range, padded to 16 bytes.
//!
//! Every cluster carries its own copy of its vertices, so a page needs nothing outside itself.
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
use crate::meshlet::{GpuMeshlet, GpuVertex};

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

/// Set in a cluster's `section` word (`GpuMeshlet::section`, above its three bytes) when its
/// payload holds packed vertices (#218); a cluster without it holds [`PagedVertex`] records,
/// as the skin pass writes them every frame.
pub const SECTION_PACKED: u32 = 1 << 24;
/// Bytes of a packed payload's header: the cluster's grid origin, then its widths and step.
pub const PACKED_HEADER_BYTES: usize = 16;
/// The widest a packed coordinate may be, in bits (the shaders shift by less than 32).
pub const PACKED_MAX_BITS: u32 = 31;

/// The exponent of the grid a mesh whose positions reach `reach` metres from its origin is
/// quantised to (#218): a step of about 2^-17 of that reach, at most a millimetre (2^-10 m) and
/// at least 2^-16 m. A power of two, so a coordinate on the grid is an integer times the step,
/// exactly.
pub fn grid_exponent(reach: f32) -> i32 {
    let exponent = (reach.max(f32::MIN_POSITIVE) / 131_072.0).log2().floor() as i32;
    exponent.clamp(-16, -10)
}

/// The step of the grid of `exponent`, in metres.
pub fn grid_step(exponent: i32) -> f32 {
    f32::from_bits(((exponent + 127) as u32) << 23)
}

/// `p` on the grid of `exponent`: each coordinate rounded to the nearest step. The cook snaps
/// every vertex before it builds the DAG, so the packed payloads hold the positions exactly.
pub fn snap(p: [f32; 3], exponent: i32) -> [f32; 3] {
    let step = grid_step(exponent);
    p.map(|c| (c / step).round() * step)
}

/// How a packed cluster stores its positions: an origin on the grid, each vertex's offset
/// from it in `bits` per axis, and the grid's exponent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packed {
    /// The cluster's smallest grid coordinates.
    pub origin: [i32; 3],
    /// Bits per offset, per axis (0 when the cluster is flat along it).
    pub bits: [u32; 3],
    /// The grid's exponent ([`grid_exponent`]).
    pub exponent: i32,
}

impl Packed {
    /// The packing of `positions` (on the grid of `exponent`).
    pub fn of(positions: impl Iterator<Item = [f32; 3]> + Clone, exponent: i32) -> Self {
        let step = grid_step(exponent);
        let grid = |p: [f32; 3]| p.map(|c| (c / step).round() as i64);
        let mut lo = [i64::MAX; 3];
        let mut hi = [i64::MIN; 3];
        for p in positions {
            let q = grid(p);
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        if lo[0] > hi[0] {
            return Self {
                origin: [0; 3],
                bits: [0; 3],
                exponent,
            };
        }
        let bits = [0, 1, 2].map(|k| 64 - ((hi[k] - lo[k]) as u64).leading_zeros());
        assert!(
            bits.iter().all(|&b| b <= PACKED_MAX_BITS),
            "a cluster {bits:?} bits wide on a grid of 2^{exponent} m"
        );
        Self {
            origin: lo.map(|v| i32::try_from(v).expect("a grid coordinate beyond 32 bits")),
            bits,
            exponent,
        }
    }

    /// Bits per vertex.
    pub fn stride(&self) -> u32 {
        self.bits.iter().sum()
    }

    /// Bytes of the positions of `vertices`: whole words, and one more so a reader may always
    /// take the word after the one an offset starts in.
    pub fn position_bytes(&self, vertices: u32) -> usize {
        (vertices as usize * self.stride() as usize).div_ceil(32) * 4 + 4
    }

    /// The header's fourth word: the widths, then the exponent biased by 128.
    pub fn info(&self) -> u32 {
        self.bits[0] | self.bits[1] << 5 | self.bits[2] << 10 | ((self.exponent + 128) as u32) << 16
    }

    /// The packing a payload's header holds.
    pub fn from_header(header: &[u8]) -> Self {
        let word = |i: usize| u32::from_le_bytes(header[4 * i..4 * i + 4].try_into().unwrap());
        let info = word(3);
        Self {
            origin: [0, 1, 2].map(|i| word(i) as i32),
            bits: [info & 31, (info >> 5) & 31, (info >> 10) & 31],
            exponent: ((info >> 16) & 0xFF) as i32 - 128,
        }
    }
}

/// Where the parts of a cluster's payload start, in bytes from its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The packing, for a packed payload.
    pub packed: Option<Packed>,
    /// The positions: [`PagedVertex`] records (normals included), or packed offsets.
    pub positions: usize,
    /// The triangles: three one-byte local indices each.
    pub triangles: usize,
    /// The UV stream, when the mesh has one (16-byte aligned, after the triangles).
    pub uvs: usize,
}

impl Layout {
    /// The layout of a payload of `vertices` and `triangles`, `packed` or of raw records. A
    /// packed payload: its header, a normal per vertex (32 bits), then the positions.
    pub fn new(vertices: u32, triangles: u32, packed: Option<Packed>) -> Self {
        let (positions, triangles_at) = match &packed {
            None => (0, vertices as usize * size_of::<PagedVertex>()),
            Some(p) => {
                let positions = PACKED_HEADER_BYTES + vertices as usize * 4;
                (positions, positions + p.position_bytes(vertices))
            }
        };
        Self {
            packed,
            positions,
            triangles: triangles_at,
            uvs: (triangles_at + triangles as usize * 3).next_multiple_of(PAYLOAD_ALIGN),
        }
    }

    /// The layout of cluster `m`'s payload, which `payload` starts with.
    pub fn of(m: &GpuMeshlet, payload: &[u8]) -> Self {
        let packed = (m.section & SECTION_PACKED != 0)
            .then(|| Packed::from_header(&payload[..PACKED_HEADER_BYTES]));
        Self::new(m.vertex_count, m.triangle_count, packed)
    }

    /// Vertex `i` of `payload`, laid out so.
    pub fn vertex(&self, payload: &[u8], i: usize) -> PagedVertex {
        let word = |at: usize| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
        let Some(p) = self.packed else {
            let at = i * size_of::<PagedVertex>();
            return bytemuck::pod_read_unaligned(&payload[at..at + size_of::<PagedVertex>()]);
        };
        let step = grid_step(p.exponent);
        let mut offset = i * p.stride() as usize;
        let position = [0, 1, 2].map(|k| {
            let at = self.positions + offset / 32 * 4;
            let pair = u64::from(word(at)) | u64::from(word(at + 4)) << 32;
            let value = (pair >> (offset % 32)) & ((1_u64 << p.bits[k]) - 1);
            offset += p.bits[k] as usize;
            p.origin[k].wrapping_add(value as i32) as f32 * step
        });
        PagedVertex {
            position,
            normal: word(PACKED_HEADER_BYTES + 4 * i),
        }
    }

    /// The local vertex indices of triangle `t` of `payload`.
    pub fn triangle(&self, payload: &[u8], t: usize) -> [u8; 3] {
        let at = self.triangles + 3 * t;
        [payload[at], payload[at + 1], payload[at + 2]]
    }
}

/// Bytes of a cluster's payload with `vertices` and `triangles`, `packed` or of raw records,
/// and its UV stream when `uvs`.
pub fn payload_bytes(vertices: u32, triangles: u32, uvs: bool, packed: Option<Packed>) -> usize {
    let stream = if uvs {
        UV_RANGE_BYTES + (vertices as usize * 4).next_multiple_of(PAYLOAD_ALIGN)
    } else {
        0
    };
    Layout::new(vertices, triangles, packed).uvs + stream
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

/// Bytes of cluster `m`'s payload, which `payload` starts with, its UV stream included when
/// `uvs`.
pub fn stored_bytes(m: &GpuMeshlet, payload: &[u8], uvs: bool) -> usize {
    payload_bytes(
        m.vertex_count,
        m.triangle_count,
        uvs,
        Layout::of(m, payload).packed,
    )
}

/// The UV of vertex `i` from the UV stream at byte `stream` of `payload` ([`Layout::uvs`]).
pub fn paged_uv(payload: &[u8], stream: usize, i: usize) -> [f32; 2] {
    let range: [f32; 4] = bytemuck::pod_read_unaligned(&payload[stream..stream + UV_RANGE_BYTES]);
    let e = stream + UV_RANGE_BYTES + i * 4;
    decode_uv(
        u32::from_le_bytes(payload[e..e + 4].try_into().unwrap()),
        range,
    )
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

/// Writes cluster `c`'s payload at `page`/`offset`, `packed` or of raw records, and records the
/// place (and the packing's flag) in its record.
#[allow(clippy::too_many_arguments)]
fn write_payload(
    dag: &mut ClusterDag,
    vertices: &[GpuVertex],
    uvs: bool,
    packed: Option<Packed>,
    bytes: &mut [u8],
    c: usize,
    page: u32,
    offset: usize,
) {
    let m = dag.meshlets[c];
    let range = dag.ranges[c];
    let local =
        |i: usize| vertices[dag.meshlet_vertices[range.vertex_offset as usize + i] as usize];
    let count = m.vertex_count as usize;
    let layout = Layout::new(m.vertex_count, m.triangle_count, packed);
    let base = page as usize * PAGE_SIZE + offset;
    let payload =
        &mut bytes[base..base + payload_bytes(m.vertex_count, m.triangle_count, uvs, packed)];
    match packed {
        None => {
            for i in 0..count {
                let v = local(i);
                let paged = PagedVertex {
                    position: v.position,
                    normal: encode_normal(v.normal),
                };
                let at = i * size_of::<PagedVertex>();
                payload[at..at + size_of::<PagedVertex>()]
                    .copy_from_slice(bytemuck::bytes_of(&paged));
            }
        }
        Some(p) => {
            for (k, word) in p
                .origin
                .map(|o| o as u32)
                .into_iter()
                .chain([p.info()])
                .enumerate()
            {
                payload[4 * k..4 * k + 4].copy_from_slice(&word.to_le_bytes());
            }
            let step = grid_step(p.exponent);
            // One word more than stored: a 0-bit axis may leave the cursor on the last one.
            let mut words = vec![0_u32; p.position_bytes(m.vertex_count) / 4 + 1];
            let mut bit = 0_usize;
            for i in 0..count {
                let v = local(i);
                let at = PACKED_HEADER_BYTES + 4 * i;
                payload[at..at + 4].copy_from_slice(&encode_normal(v.normal).to_le_bytes());
                for k in 0..3 {
                    let grid = v.position[k] / step;
                    assert_eq!(grid.round(), grid, "a vertex off its mesh's grid");
                    let value = (grid as i64 - i64::from(p.origin[k])) as u64;
                    let word = bit / 32;
                    let shifted = value << (bit % 32);
                    words[word] |= shifted as u32;
                    words[word + 1] |= (shifted >> 32) as u32;
                    bit += p.bits[k] as usize;
                }
            }
            for (k, word) in words[..words.len() - 1].iter().enumerate() {
                let at = layout.positions + 4 * k;
                payload[at..at + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
    }
    let t = range.triangle_offset as usize;
    let n = m.triangle_count as usize * 3;
    payload[layout.triangles..layout.triangles + n]
        .copy_from_slice(&dag.meshlet_triangles[t..t + n]);
    if uvs {
        let uv_range = uv_range((0..count).map(|i| local(i).uv));
        let mut at = layout.uvs;
        payload[at..at + UV_RANGE_BYTES].copy_from_slice(bytemuck::bytes_of(&uv_range));
        at += UV_RANGE_BYTES;
        for i in 0..count {
            let e = encode_uv(local(i).uv, uv_range);
            payload[at..at + 4].copy_from_slice(&e.to_le_bytes());
            at += 4;
        }
    }
    dag.meshlets[c].page = page;
    dag.meshlets[c].payload = offset as u32;
    if packed.is_some() {
        dag.meshlets[c].section |= SECTION_PACKED;
    }
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
/// `page`, `payload` and `child_page`. With `uvs` every payload carries its UV stream. With
/// `grid`, the exponent of the grid the vertices were snapped to ([`snap`]), the payloads hold
/// packed vertices (#218); without, [`PagedVertex`] records (the skinned meshes').
pub fn pack(dag: &mut ClusterDag, vertices: &[GpuVertex], uvs: bool, grid: Option<i32>) -> Pages {
    let n = dag.meshlets.len();
    let packings: Vec<Option<Packed>> = (0..n)
        .map(|c| {
            grid.map(|exponent| {
                let range = dag.ranges[c];
                let first = range.vertex_offset as usize;
                let local =
                    &dag.meshlet_vertices[first..first + dag.meshlets[c].vertex_count as usize];
                Packed::of(
                    local.iter().map(|&v| vertices[v as usize].position),
                    exponent,
                )
            })
        })
        .collect();
    let size = |dag: &ClusterDag, c: usize| {
        payload_bytes(
            dag.meshlets[c].vertex_count,
            dag.meshlets[c].triangle_count,
            uvs,
            packings[c],
        )
    };
    let mut packer = Packer {
        bytes: Vec::new(),
        cursor: PAGE_SIZE,
    };

    // The roots, cluster by cluster.
    for (c, &packed) in packings.iter().enumerate() {
        if dag.meshlets[c].parent_error.is_infinite() {
            let (page, offset) = packer.reserve(size(dag, c));
            write_payload(
                dag,
                vertices,
                uvs,
                packed,
                &mut packer.bytes,
                c,
                page,
                offset,
            );
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
            write_payload(
                dag,
                vertices,
                uvs,
                packings[c],
                &mut packer.bytes,
                c,
                page,
                offset,
            );
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

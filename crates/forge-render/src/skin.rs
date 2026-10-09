//! Skinned meshes in the frame (#165, `shaders/skin.slang`): bodies that bend at their joints.
//!
//! A skinned mesh ([`forge_geom::SkinnedMesh`]) is cooked as one level of clusters, all roots,
//! whose pages are always resident; its instance is a mover (#79), which places it, and
//! [`crate::MeshletScene::set_skins`] gives every frame its joints' matrices in the mover's
//! frame. The pass `skin/vertices` then rewrites the clusters' vertices in the pool of pages
//! before the culls read them, so the draws, the visibility buffer and the shading see the bent
//! body as any other mesh; its clusters' spheres hold every pose and their normal cones are
//! off, so culling stays right whatever the pose.
//!
//! With rays, the same pass writes the positions into the ray tracing's copy of the mesh's cut
//! (all its clusters, in the same order), and `skin/blas` refits the mesh's bottom-level
//! structure from them ([`forge_gpu::DynamicBlas`]) before the movers' top-level one is built
//! over it. The pass also writes where the frame before's matrices put each vertex, from which
//! `mover_motion_main` gives the bent body's pixels their own motion.
//!
//! A mesh blends its vertices' joints by their matrices (linear blend skinning), or, on
//! request, as dual quaternions (#169, D-052, [`SkinBlend`]), which keep a twisting joint's
//! thickness. [`skin_vertex`] is the pass's twin on the CPU.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_geom::{GpuMeshlet, PAGE_SIZE, SkinVertex};
use forge_gpu::{
    Buffer, BufferDesc, Device, GraphBuffer, MemoryCategory, MemoryLocation, Result, vk,
};
use glam::{Mat4, Quat, Vec3, Vec4};

/// How a skinned mesh blends each vertex's joints (#169, D-052).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SkinBlend {
    /// Their matrices, weighted (linear blend skinning). A joint that twists thins the limb
    /// about it (the "candy wrapper").
    #[default]
    Linear,
    /// Their rigid motions as unit dual quaternions, weighted and normalised (Kavan et al.,
    /// 2007): a twisting joint keeps its thickness, a bending one swells a little. Each matrix's
    /// uniform scale (the length of its first row) is blended apart and applied first.
    DualQuaternion,
}

/// `GpuSkinCluster::flags`: the cluster's mesh blends as dual quaternions.
const SKIN_DUAL_QUATERNION: u32 = 1;

/// Where the skin pass puts a bind-pose vertex at `position` with `normal`, carried by the
/// joints `joints` (indices into `matrices`) weighted by `weights`, blended by `blend`: the twin
/// of `skin_main` in `shaders/skin.slang`, for tests and measurements.
pub fn skin_vertex(
    matrices: &[Mat4],
    joints: [u16; 4],
    weights: [f32; 4],
    position: Vec3,
    normal: Vec3,
    blend: SkinBlend,
) -> (Vec3, Vec3) {
    let used = || {
        (0..4)
            .filter(move |&k| weights[k] > 0.0)
            .map(move |k| (matrices[usize::from(joints[k])], weights[k]))
    };
    match blend {
        SkinBlend::Linear => {
            let m = used().fold(Mat4::ZERO, |sum, (m, w)| sum + m * w);
            let n = m.transform_vector3(normal).normalize_or(normal);
            (m.transform_point3(position), n)
        }
        SkinBlend::DualQuaternion => {
            let mut pivot = None;
            let (mut real, mut dual, mut scale) = (Vec4::ZERO, Vec4::ZERO, 0.0);
            for (m, w) in used() {
                let (q, d, s) = dual_quaternion(m);
                let first = *pivot.get_or_insert(q);
                // The shorter way round: against the first joint's rotation.
                let sign = if q.dot(first) < 0.0 { -1.0 } else { 1.0 };
                real += w * sign * Vec4::from(q);
                dual += w * sign * d;
                scale += w * s;
            }
            let length = real.length();
            let (r, d) = (Quat::from_vec4(real / length), dual / length);
            // The translation the unit dual quaternion holds: 2 (d r*).
            let rv = Vec3::new(r.x, r.y, r.z);
            let dv = d.truncate();
            let t = 2.0 * (r.w * dv - d.w * rv + rv.cross(dv));
            (
                (r * (scale * position)) + t,
                (r * normal).normalize_or(normal),
            )
        }
    }
}

/// A rigid matrix (rotation, translation, a uniform scale) as a unit dual quaternion, its real
/// part the rotation and its dual part `t r / 2`, and its scale.
fn dual_quaternion(m: Mat4) -> (Quat, Vec4, f32) {
    let rows = [m.row(0), m.row(1), m.row(2)];
    let scale = rows[0].truncate().length();
    let r = glam::Mat3::from_cols(
        m.x_axis.truncate() / scale,
        m.y_axis.truncate() / scale,
        m.z_axis.truncate() / scale,
    );
    let q = Quat::from_mat3(&r).normalize();
    let t = Quat::from_xyzw(rows[0].w, rows[1].w, rows[2].w, 0.0);
    (q, 0.5 * Vec4::from(t * q), scale)
}

/// Mirrors `SkinCluster` in `skin.slang` (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuSkinCluster {
    /// Bytes into the pool where its payload starts.
    pool: u32,
    /// Its first vertex in the skin vertices and the previous positions.
    first: u32,
    vertex_count: u32,
    /// Its first vertex in the ray tracing's positions (`u32::MAX`: none).
    ray: u32,
    /// Its mesh's first matrix in a frame's joints.
    joints: u32,
    /// Its mesh's height field in the fields' table (`u32::MAX`: none).
    field: u32,
    /// `SKIN_DUAL_QUATERNION` when its mesh blends as dual quaternions (#169).
    flags: u32,
    pad: u32,
}

/// A height field that raises a displaced mesh's vertices (#185's deformable ground, added with
/// [`crate::MeshletSceneBuilder::add_displaced_mesh`]): a grid of heights over the mesh's
/// bind-pose x and z, which [`crate::MeshletScene::set_fields`] gives every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightField {
    /// Where its first height lies, (x, z) in the mesh's bind pose.
    pub origin: [f32; 2],
    /// Metres between two heights.
    pub cell: f32,
    /// Heights along x and z.
    pub size: [u32; 2],
    /// Whether its heights are followed by a slope per point (x then z, as `dy/dx`), added to
    /// the slope its heights make there (#197): a ground window's, which turns the facets of
    /// the ground it raises into the ground's own smooth normals.
    pub slopes: bool,
}

/// Mirrors `HeightField` in `skin.slang` (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuHeightField {
    origin: [f32; 2],
    cell: f32,
    /// Its first height in a frame's heights.
    first: u32,
    size: [u32; 2],
    /// Its first slope in a frame's heights, two a point (`u32::MAX`: none).
    slopes: u32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuHeightField>() == 32);

const _: () = assert!(std::mem::size_of::<GpuSkinCluster>() == 32);

/// Mirrors `SkinPush` in `skin.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct SkinPush {
    pub clusters: u64,
    pub vertices: u64,
    pub joints: u64,
    pub previous_joints: u64,
    pub pool: u64,
    pub rays: u64,
    pub previous: u64,
    pub fields: u64,
    pub heights: u64,
    pub previous_heights: u64,
    pub cluster_count: u32,
    pub has_rays: u32,
}

/// A skinned mesh as the scene builder keeps it until the build.
pub(crate) struct SkinSource {
    /// Its index in the mesh table.
    pub mesh: u32,
    /// Its first cluster in the scene's cluster table, and how many.
    pub meshlet_offset: u32,
    pub meshlet_count: u32,
    /// Its joints.
    pub joints: u32,
    /// Per cluster vertex, in the pages' order.
    pub vertices: Vec<SkinVertex>,
    /// The height field that raises it, for a displaced mesh.
    pub field: Option<HeightField>,
    /// How it blends its vertices' joints (#169).
    pub blend: SkinBlend,
}

/// Ring slots of the joints' matrices: this frame's, the frame before's and one more, as the
/// movers' (`MOVER_SLOTS`).
const SKIN_SLOTS: usize = forge_gpu::FRAMES_IN_FLIGHT + 1;
/// Bytes of a joint's matrix on the GPU: three rows of four floats.
const JOINT_BYTES: u64 = 48;

/// A scene's skinned meshes.
pub(crate) struct SceneSkins {
    clusters: Buffer,
    vertices: Buffer,
    /// Where the frame before's matrices put each vertex (three floats a vertex): written by
    /// `skin/vertices`, read by the motion vectors.
    pub previous: GraphBuffer,
    /// The joints' matrices of each frame, every skinned mesh's in the order they were added.
    ring: Vec<Buffer>,
    /// Matrices a frame.
    joint_count: u32,
    pub cluster_count: u32,
    /// Per skinned mesh: its index in the mesh table and its first cluster in `clusters`.
    pub meshes: Vec<(u32, u32)>,
    /// The ray tracing's positions are written.
    pub has_rays: bool,
    /// The joints' ring slots.
    turn: Turn,
    /// The displaced meshes' height fields (`GpuHeightField`, a record at least).
    fields: Buffer,
    /// The heights of each frame, every height field's in the order they were added.
    heights: Vec<Buffer>,
    /// Heights a frame.
    height_count: u32,
    /// The heights' ring slots.
    heights_turn: Turn,
    /// Whether joints or heights came since the skin pass last ran (#197): it runs only then.
    fresh: Cell<bool>,
}

/// Which slots of a ring this frame's data and the frame before's are in.
#[derive(Default)]
struct Turn {
    current: Cell<usize>,
    previous: Cell<usize>,
    written: Cell<u64>,
}

impl Turn {
    /// The slot the next frame's data goes to, which becomes the current one; the current one
    /// becomes the frame before's (the same one at the first frame).
    fn advance(&self) -> usize {
        let slot = (self.written.get() % SKIN_SLOTS as u64) as usize;
        let first_frame = self.written.get() == 0;
        self.previous.set(if first_frame {
            slot
        } else {
            self.current.get()
        });
        self.current.set(slot);
        self.written.set(self.written.get() + 1);
        slot
    }
}

impl SceneSkins {
    /// The skinned meshes of `sources` over the scene's clusters `meshlets` (their pages
    /// rebased into the scene's): `slot` gives a page's slot in the pool, `ray_first` a mesh's
    /// first vertex in the ray tracing's positions (none without rays).
    pub fn new(
        device: &Arc<Device>,
        sources: &[SkinSource],
        meshlets: &[GpuMeshlet],
        slot: impl Fn(u32) -> u32,
        ray_first: impl Fn(u32) -> Option<u32>,
    ) -> Result<Self> {
        let mut clusters = Vec::new();
        let mut vertices = Vec::new();
        let mut meshes = Vec::new();
        let mut joints = 0;
        let mut has_rays = false;
        let mut fields = Vec::new();
        let mut heights = 0;
        for source in sources {
            meshes.push((source.mesh, clusters.len() as u32));
            let ray = ray_first(source.mesh);
            has_rays |= ray.is_some();
            let field = source.field.map_or(u32::MAX, |f| {
                let points = f.size[0] * f.size[1];
                fields.push(GpuHeightField {
                    origin: f.origin,
                    cell: f.cell,
                    first: heights,
                    size: f.size,
                    slopes: if f.slopes { heights + points } else { u32::MAX },
                    pad: 0,
                });
                heights += points * if f.slopes { 3 } else { 1 };
                fields.len() as u32 - 1
            });
            let mut local = 0;
            let first = source.meshlet_offset as usize;
            for m in &meshlets[first..first + source.meshlet_count as usize] {
                clusters.push(GpuSkinCluster {
                    pool: slot(m.page) * PAGE_SIZE as u32 + m.payload,
                    first: (vertices.len() + local) as u32,
                    vertex_count: m.vertex_count,
                    ray: ray.map_or(u32::MAX, |r| r + local as u32),
                    joints,
                    field,
                    flags: if source.blend == SkinBlend::DualQuaternion {
                        SKIN_DUAL_QUATERNION
                    } else {
                        0
                    },
                    pad: 0,
                });
                local += m.vertex_count as usize;
            }
            assert_eq!(
                local,
                source.vertices.len(),
                "a skin vertex per cluster vertex"
            );
            vertices.extend_from_slice(&source.vertices);
            joints += source.joints;
        }
        assert!(
            clusters.len() <= 65_535,
            "the skin pass dispatches a workgroup per cluster, at most 65 535"
        );
        tracing::info!(
            meshes = sources.len(),
            joints,
            clusters = clusters.len(),
            vertices = vertices.len(),
            fields = fields.len(),
            heights,
            "skinned meshes ready"
        );
        let storage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let ring = (0..SKIN_SLOTS)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: u64::from(joints.max(1)) * JOINT_BYTES,
                    usage: storage,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("skin joints {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        // Until the first frame's matrices come, the bind pose.
        let identity = vec![rows(Mat4::IDENTITY); joints as usize];
        for buffer in &ring {
            buffer.write(0, &identity);
        }
        // Until the first frame's heights come, flat.
        let heights_ring = (0..SKIN_SLOTS)
            .map(|i| {
                let buffer = device.create_buffer(BufferDesc {
                    size: u64::from(heights.max(1)) * 4,
                    usage: storage,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("skin heights {i}"),
                })?;
                buffer.write(0, &vec![0.0f32; heights as usize]);
                Ok(buffer)
            })
            .collect::<Result<Vec<_>>>()?;
        if fields.is_empty() {
            fields.push(GpuHeightField::zeroed());
        }
        Ok(Self {
            clusters: device.create_buffer_with_data(
                &clusters,
                storage,
                MemoryCategory::Geometry,
                "skin clusters",
            )?,
            vertices: device.create_buffer_with_data(
                &vertices,
                storage,
                MemoryCategory::Geometry,
                "skin vertices",
            )?,
            previous: GraphBuffer::new(device.create_buffer(BufferDesc {
                size: (vertices.len().max(1) * 12) as u64,
                usage: storage,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Geometry,
                name: "skin previous positions",
            })?),
            ring,
            joint_count: joints,
            cluster_count: clusters.len() as u32,
            meshes,
            has_rays,
            turn: Turn::default(),
            fields: device.create_buffer_with_data(
                &fields,
                storage,
                MemoryCategory::Geometry,
                "skin height fields",
            )?,
            heights: heights_ring,
            height_count: heights,
            heights_turn: Turn::default(),
            // The first frame bends the bind pose once, the previous positions with it.
            fresh: Cell::new(true),
        })
    }

    /// Writes a frame's matrices, every skinned mesh's joints in turn, into the next slot of
    /// the ring.
    pub fn set(&self, matrices: &[Mat4]) {
        assert_eq!(
            matrices.len(),
            self.joint_count as usize,
            "a matrix per joint of every skinned mesh"
        );
        let data: Vec<[f32; 12]> = matrices.iter().map(|&m| rows(m)).collect();
        self.ring[self.turn.advance()].write(0, &data);
        self.fresh.set(true);
    }

    /// Writes a frame's heights, every height field's in turn (row by row along x), into the
    /// next slot of their ring.
    pub fn set_fields(&self, heights: &[f32]) {
        assert_eq!(
            heights.len(),
            self.height_count as usize,
            "a height per point of every height field"
        );
        self.heights[self.heights_turn.advance()].write(0, heights);
        self.fresh.set(true);
    }

    /// Whether joints or heights came since the last call (#197): the skin pass and the
    /// structures' refit run only then. What they wrote stays: the pool's pages of skinned
    /// clusters are roots, always resident.
    pub fn take_fresh(&self) -> bool {
        self.fresh.replace(false)
    }

    /// The pass's push constants: the pool at `pool`, the ray tracing's positions at `rays`.
    pub fn push(&self, pool: u64, rays: u64) -> SkinPush {
        SkinPush {
            clusters: self.clusters.address(),
            vertices: self.vertices.address(),
            joints: self.ring[self.turn.current.get()].address(),
            previous_joints: self.ring[self.turn.previous.get()].address(),
            pool,
            rays,
            previous: self.previous.address(),
            fields: self.fields.address(),
            heights: self.heights[self.heights_turn.current.get()].address(),
            previous_heights: self.heights[self.heights_turn.previous.get()].address(),
            cluster_count: self.cluster_count,
            has_rays: u32::from(self.has_rays && rays != 0),
        }
    }

    /// The clusters' records (`SkinCluster`), which the motion vectors and the resolve read.
    pub fn clusters_address(&self) -> u64 {
        self.clusters.address()
    }

    /// This frame's joints' matrices (the ring slot [`SceneSkins::set`] wrote last).
    pub fn joints_address(&self) -> u64 {
        self.ring[self.turn.current.get()].address()
    }

    /// The cluster vertices in the bind pose (`SkinVertex`), from which the resolve projects the
    /// skinned meshes' textures (#166).
    pub fn vertices_address(&self) -> u64 {
        self.vertices.address()
    }
}

/// A matrix's first three rows, as the shader reads it.
fn rows(m: Mat4) -> [f32; 12] {
    let mut out = [0.0; 12];
    for r in 0..3 {
        out[4 * r..4 * r + 4].copy_from_slice(&m.row(r).to_array());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_apply_as_the_matrix_does() {
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(1.5),
            Quat::from_rotation_y(0.7),
            Vec3::new(1.0, -2.0, 3.0),
        );
        let r = rows(m);
        let p = Vec4::new(0.3, 0.4, -0.5, 1.0);
        let moved = Vec3::new(
            Vec4::from_slice(&r[0..4]).dot(p),
            Vec4::from_slice(&r[4..8]).dot(p),
            Vec4::from_slice(&r[8..12]).dot(p),
        );
        assert!(moved.abs_diff_eq(m.transform_point3(p.truncate()), 1e-6));
    }

    #[test]
    fn one_joint_moves_a_vertex_as_its_matrix_does_either_way() {
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(1.5),
            Quat::from_euler(glam::EulerRot::YXZ, 0.7, -0.4, 2.9),
            Vec3::new(1.0, -2.0, 3.0),
        );
        let (p, n) = (Vec3::new(0.3, 0.4, -0.5), Vec3::new(0.0, 0.6, 0.8));
        for blend in [SkinBlend::Linear, SkinBlend::DualQuaternion] {
            let (at, normal) = skin_vertex(&[m], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0], p, n, blend);
            assert!(
                at.abs_diff_eq(m.transform_point3(p), 1e-5),
                "{blend:?}: {at}"
            );
            let turned = m.transform_vector3(n).normalize();
            assert!(normal.abs_diff_eq(turned, 1e-5), "{blend:?}: {normal}");
        }
    }

    #[test]
    fn a_twist_thins_a_linear_blend_and_keeps_a_dual_quaternion_s_radius() {
        // Two joints along x, the second turned 90° about the axis, a vertex 5 cm from the
        // axis shared half and half: the linear blend pulls it in to cos 45° of its distance.
        let twist = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let matrices = [Mat4::IDENTITY, twist];
        let p = Vec3::new(0.2, 0.05, 0.0);
        let n = Vec3::Y;
        let radius = |v: Vec3| (v.y * v.y + v.z * v.z).sqrt();
        let half = [0.5, 0.5, 0.0, 0.0];
        let (linear, _) = skin_vertex(&matrices, [0, 1, 0, 0], half, p, n, SkinBlend::Linear);
        let (dual, normal) = skin_vertex(
            &matrices,
            [0, 1, 0, 0],
            half,
            p,
            n,
            SkinBlend::DualQuaternion,
        );
        assert!((radius(linear) - 0.05 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!((radius(dual) - 0.05).abs() < 1e-5, "{dual}");
        assert!(
            (dual.x - 0.2).abs() < 1e-6,
            "it stays where it was along the axis"
        );
        // Half way round: turned 45°, its normal too.
        let angle = dual.z.atan2(dual.y);
        assert!(
            (angle - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "{angle}"
        );
        assert!(normal.abs_diff_eq(Vec3::new(0.0, dual.y, dual.z).normalize(), 1e-5));
        // A joint's quaternion and its negation are the same turn: the blend takes the shorter
        // way round whichever comes.
        let full = Mat4::from_rotation_x(std::f32::consts::PI);
        let (a, _) = skin_vertex(
            &[Mat4::IDENTITY, full],
            [0, 1, 0, 0],
            [0.75, 0.25, 0.0, 0.0],
            p,
            n,
            SkinBlend::DualQuaternion,
        );
        assert!((radius(a) - 0.05).abs() < 1e-5, "{a}");
    }
}

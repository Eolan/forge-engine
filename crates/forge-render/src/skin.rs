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

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_geom::{GpuMeshlet, PAGE_SIZE, SkinVertex};
use forge_gpu::{
    Buffer, BufferDesc, Device, GraphBuffer, MemoryCategory, MemoryLocation, Result, vk,
};
use glam::Mat4;

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
    pad: [u32; 3],
}

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
    current: Cell<usize>,
    previous_slot: Cell<usize>,
    written: Cell<u64>,
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
        for source in sources {
            meshes.push((source.mesh, clusters.len() as u32));
            let ray = ray_first(source.mesh);
            has_rays |= ray.is_some();
            let mut local = 0;
            let first = source.meshlet_offset as usize;
            for m in &meshlets[first..first + source.meshlet_count as usize] {
                clusters.push(GpuSkinCluster {
                    pool: slot(m.page) * PAGE_SIZE as u32 + m.payload,
                    first: (vertices.len() + local) as u32,
                    vertex_count: m.vertex_count,
                    ray: ray.map_or(u32::MAX, |r| r + local as u32),
                    joints,
                    pad: [0; 3],
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
            current: Cell::new(0),
            previous_slot: Cell::new(0),
            written: Cell::new(0),
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
        let slot = (self.written.get() % SKIN_SLOTS as u64) as usize;
        let data: Vec<[f32; 12]> = matrices.iter().map(|&m| rows(m)).collect();
        self.ring[slot].write(0, &data);
        let first_frame = self.written.get() == 0;
        self.previous_slot.set(if first_frame {
            slot
        } else {
            self.current.get()
        });
        self.current.set(slot);
        self.written.set(self.written.get() + 1);
    }

    /// The pass's push constants: the pool at `pool`, the ray tracing's positions at `rays`.
    pub fn push(&self, pool: u64, rays: u64) -> SkinPush {
        SkinPush {
            clusters: self.clusters.address(),
            vertices: self.vertices.address(),
            joints: self.ring[self.current.get()].address(),
            previous_joints: self.ring[self.previous_slot.get()].address(),
            pool,
            rays,
            previous: self.previous.address(),
            cluster_count: self.cluster_count,
            has_rays: u32::from(self.has_rays && rays != 0),
        }
    }

    /// The clusters' records (`SkinCluster`), which the motion vectors read.
    pub fn clusters_address(&self) -> u64 {
        self.clusters.address()
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
    use glam::{Quat, Vec3, Vec4};

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
}

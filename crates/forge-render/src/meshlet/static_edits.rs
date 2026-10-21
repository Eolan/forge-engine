//! Static instances changed at run time (#323): a door's leaf swung, a sash raised, a tree's far
//! stand-in swapped for the tree, one hidden or shown.
//!
//! A scene built with [`super::MeshletSceneBuilder::allow_static_edits`] keeps its static
//! instances' records on the CPU. [`super::MeshletScene::set_static_transform`] and
//! [`super::MeshletScene::set_static_visibility`] change a record and mark it; the next frame
//! drawn copies the marked records into the instance table from a host-visible slot of its own
//! (`scene/static edits`), redoes the culling cells that hold them (`scene/static cell bounds`),
//! and has the static acceleration structure rewrite their records and update itself in place
//! (`scene/static tlas`, [`crate::raytrace::SceneRays::declare_static_update`]). Frames without
//! edits add nothing.
//!
//! Not yet: a moved static instance's own motion for the temporal passes (it gets the camera's,
//! as before; a door that swings over half a second belongs to the movers), and the probes' wake
//! (#79) for what it uncovers.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, MemoryCategory, MemoryLocation, Result, vk,
};
use glam::{IVec3, Quat, Vec3, Vec4};

use super::{GpuInstance, GpuMesh, INSTANCE_VISIBILITY, MoverTransform, Visibility};
use crate::cells::CellPos;

/// The most records a frame uploads; the rest wait for the next frames.
pub(super) const EDITS_PER_FRAME: usize = 4096;

/// A scene's static instances as the CPU changes them.
pub(super) struct StaticEdits {
    /// The static instances' records, edits included.
    records: RefCell<Vec<GpuInstance>>,
    /// Per mesh, its bounding sphere in object space (centre, radius).
    bounds: Vec<Vec4>,
    /// The records changed since a frame last uploaded them.
    dirty: RefCell<BTreeSet<u32>>,
    /// Per frame slot, room for [`EDITS_PER_FRAME`] records the GPU copies from.
    staging: Vec<Buffer>,
}

/// What a frame uploads: the copies from its slot into the table (source offset, destination
/// offset, bytes), and the first and last instance they touch.
pub(super) struct StagedEdits {
    pub regions: Vec<(u64, u64, u64)>,
    pub first: u32,
    pub last: u32,
}

impl StaticEdits {
    pub(super) fn new(
        device: &Arc<Device>,
        records: &[GpuInstance],
        meshes: &[GpuMesh],
    ) -> Result<Self> {
        let size = std::mem::size_of::<GpuInstance>();
        let staging = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: (EDITS_PER_FRAME * size) as u64,
                    usage: vk::BufferUsageFlags::TRANSFER_SRC,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("static edits {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            records: RefCell::new(records.to_vec()),
            bounds: meshes
                .iter()
                .map(|m| Vec3::from(m.center).extend(m.radius))
                .collect(),
            dirty: RefCell::new(BTreeSet::new()),
            staging,
        })
    }

    /// How many static instances there are.
    pub(super) fn len(&self) -> u32 {
        self.records.borrow().len() as u32
    }

    /// Puts static instance `instance` at `transform`, from the scene's `origin`.
    pub(super) fn set_transform(&self, origin: CellPos, instance: u32, transform: MoverTransform) {
        let mut records = self.records.borrow_mut();
        let record = &mut records[instance as usize];
        let bounds = self.bounds[record.mesh as usize];
        let position = origin.offset(transform.position);
        let center = transform.rotation * (bounds.truncate() * transform.scale) + position.local;
        record.cell = position.cell.to_array();
        record.local = position.local.to_array();
        record.scale = transform.scale;
        record.rotation = transform.rotation.to_array();
        record.center = center.to_array();
        record.radius = bounds.w * transform.scale;
        self.dirty.borrow_mut().insert(instance);
    }

    /// Changes what sees static instance `instance`.
    pub(super) fn set_visibility(&self, instance: u32, visibility: Visibility) {
        self.set_visibility_flags(instance, visibility.flags());
    }

    /// [`Self::set_visibility`] from the flags' bits (`INSTANCE_VISIBILITY`'s).
    pub(super) fn set_visibility_flags(&self, instance: u32, bits: u32) {
        let mut records = self.records.borrow_mut();
        let record = &mut records[instance as usize];
        record.flags = (record.flags & !INSTANCE_VISIBILITY) | (bits & INSTANCE_VISIBILITY);
        self.dirty.borrow_mut().insert(instance);
    }

    /// Static instance `instance`'s record as the edits leave it (#338's promotions).
    pub(super) fn record(&self, instance: u32) -> GpuInstance {
        self.records.borrow()[instance as usize]
    }

    /// Static instance `instance`'s transform as the edits leave it, from the scene's `origin`.
    pub(super) fn transform(&self, origin: CellPos, instance: u32) -> MoverTransform {
        let record = self.record(instance);
        let at = CellPos {
            cell: IVec3::from_array(record.cell),
            local: Vec3::from_array(record.local),
        };
        MoverTransform {
            position: at.relative_to(origin),
            rotation: Quat::from_array(record.rotation),
            scale: record.scale,
        }
    }

    /// Mesh `mesh`'s bounding sphere in object space (centre, radius).
    pub(super) fn mesh_bounds(&self, mesh: u32) -> Vec4 {
        self.bounds[mesh as usize]
    }

    /// Writes up to [`EDITS_PER_FRAME`] changed records into frame slot `slot`'s staging, the
    /// lowest first; `None` when nothing changed.
    pub(super) fn stage(&self, slot: usize) -> Option<StagedEdits> {
        let mut dirty = self.dirty.borrow_mut();
        if dirty.is_empty() {
            return None;
        }
        let records = self.records.borrow();
        let size = std::mem::size_of::<GpuInstance>() as u64;
        let taken: Vec<u32> = dirty.iter().copied().take(EDITS_PER_FRAME).collect();
        for i in &taken {
            dirty.remove(i);
        }
        let batch: Vec<GpuInstance> = taken.iter().map(|&i| records[i as usize]).collect();
        self.staging[slot].write(0, &batch);
        // One copy a run of consecutive instances (a forest added in one go is one run).
        let mut regions: Vec<(u64, u64, u64)> = Vec::new();
        for (k, &i) in taken.iter().enumerate() {
            match regions.last_mut() {
                Some((_, at, bytes)) if *at + *bytes == u64::from(i) * size => *bytes += size,
                _ => regions.push((k as u64 * size, u64::from(i) * size, size)),
            }
        }
        Some(StagedEdits {
            regions,
            first: taken[0],
            last: *taken.last().expect("at least one edit"),
        })
    }

    /// Frame slot `slot`'s staging.
    pub(super) fn staging(&self, slot: usize) -> &Buffer {
        &self.staging[slot]
    }
}

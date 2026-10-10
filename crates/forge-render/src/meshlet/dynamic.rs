//! A scene that takes and frees meshes as it runs (#220; `docs/research/planet-terrain.md`, "The
//! scene that takes and frees tiles in place").
//!
//! - **Room.** [`MeshletSceneBuilder::reserve_dynamic`] makes every table at a capacity: the
//!   cluster records, the mesh records, the instances, the page numbers, the rays' cuts. A
//!   shared [`Room`] hands out their free ranges, first fit.
//! - **The worker's side** ([`SceneEditor`], from [`MeshletScene::editor`]): a new mesh's
//!   cluster records, its record and its rays' cut go into ranges no frame reads, through
//!   staged copies beside the frames; its structure is built on the compute queue. Its root
//!   pages and the pages its first view wants are read there too, and the next top-level
//!   structure is built over the instances the edit leaves (a vacant slot masked out).
//!   [`SceneEditor::finish`] hands all this over as a [`SceneEdit`].
//! - **The frame's side** ([`MeshletScene::apply`], then the next frame's start): the pages join
//!   the residency, their roots pinned over the least needed pages and their first view's pages
//!   placed as room allows; the frame's "scene/edit" pass copies them, their page-table entries
//!   and the instances' records (a removed one vacant) before the culls; the new top-level
//!   structure takes over. Nothing moves, so the frames' lists stay valid.
//! - **Leaving.** What a removed instance or mesh held (its pages, its ranges, its structure,
//!   what it kept alive) is freed `FRAMES_IN_FLIGHT + 1` frames after the frame that removed it,
//!   when no frame reads it and the needs read back no longer name its pages.

use std::any::Any;
use std::borrow::Cow;
use std::sync::Mutex;

use super::*;
use crate::raytrace::{Cut, RayTables};
use crate::streaming::{PageEdit, ResidentSet, read_sources};
use forge_gpu::{AccelerationStructure, BlasTriangles};

/// What a dynamic scene reserves room for ([`MeshletSceneBuilder::reserve_dynamic`], #220).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicCapacity {
    /// Meshes at once.
    pub meshes: u32,
    /// Instances at once.
    pub instances: u32,
    /// Cluster records over all meshes.
    pub meshlets: u32,
    /// Cluster pages over all meshes.
    pub pages: u32,
    /// With rays: the vertices of the meshes' cuts over all of them.
    pub ray_vertices: u32,
    /// With rays: the triangles of the meshes' cuts over all of them.
    pub ray_triangles: u32,
}

/// A table's free ranges, first fit.
#[derive(Debug)]
struct Ranges {
    /// (start, length), by start, none touching another.
    free: Vec<(u32, u32)>,
    /// One past the highest entry ever handed out.
    high: u32,
}

impl Ranges {
    /// A table of `capacity` entries whose first `used` are taken.
    fn new(capacity: u32, used: u32) -> Self {
        Self {
            free: if used < capacity {
                vec![(used, capacity - used)]
            } else {
                Vec::new()
            },
            high: used,
        }
    }

    /// The first free range of `len` entries; `None` when none is that long.
    fn take(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let i = self.free.iter().position(|&(_, l)| l >= len)?;
        let (start, l) = self.free[i];
        if l == len {
            self.free.remove(i);
        } else {
            self.free[i] = (start + len, l - len);
        }
        self.high = self.high.max(start + len);
        Some(start)
    }

    /// Gives back `len` entries from `start`.
    fn give(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        let i = self.free.partition_point(|&(s, _)| s < start);
        self.free.insert(i, (start, len));
        if i + 1 < self.free.len() && start + len == self.free[i + 1].0 {
            self.free[i].1 += self.free[i + 1].1;
            self.free.remove(i + 1);
        }
        if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == start {
            self.free[i - 1].1 += self.free[i].1;
            self.free.remove(i);
        }
    }
}

/// Every table's free ranges, shared by the scene (which gives back) and its editor (which
/// takes).
#[derive(Debug)]
struct Room {
    meshes: Ranges,
    instances: Ranges,
    meshlets: Ranges,
    pages: Ranges,
    ray_vertices: Ranges,
    ray_triangles: Ranges,
}

/// Where a mesh lies in the tables: (first, count) of its cluster records, its pages, its
/// rays' vertices and triangles.
#[derive(Clone, Copy, Debug, Default)]
struct MeshRanges {
    meshlets: (u32, u32),
    pages: (u32, u32),
    ray_vertices: (u32, u32),
    ray_triangles: (u32, u32),
}

impl Room {
    /// The tables a mesh takes ranges of beside its slot: its cluster records, its pages, its
    /// rays' vertices and triangles.
    fn mesh_table(&mut self, k: usize) -> &mut Ranges {
        match k {
            0 => &mut self.meshlets,
            1 => &mut self.pages,
            2 => &mut self.ray_vertices,
            _ => &mut self.ray_triangles,
        }
    }

    /// A mesh slot and ranges of `sizes` entries of each of [`Room::mesh_table`]'s tables; the
    /// name of the first table without room otherwise, nothing taken.
    fn take_mesh(
        &mut self,
        sizes: [u32; 4],
    ) -> std::result::Result<(u32, MeshRanges), &'static str> {
        const NAMES: [&str; 4] = [
            "cluster records",
            "pages",
            "rays' vertices",
            "rays' triangles",
        ];
        let Some(slot) = self.meshes.take(1) else {
            return Err("meshes");
        };
        let mut firsts = [0; 4];
        for k in 0..4 {
            match self.mesh_table(k).take(sizes[k]) {
                Some(first) => firsts[k] = first,
                None => {
                    self.meshes.give(slot, 1);
                    for j in 0..k {
                        self.mesh_table(j).give(firsts[j], sizes[j]);
                    }
                    return Err(NAMES[k]);
                }
            }
        }
        Ok((
            slot,
            MeshRanges {
                meshlets: (firsts[0], sizes[0]),
                pages: (firsts[1], sizes[1]),
                ray_vertices: (firsts[2], sizes[2]),
                ray_triangles: (firsts[3], sizes[3]),
            },
        ))
    }

    /// Gives back mesh slot `slot`'s ranges.
    fn give_mesh(&mut self, slot: u32, r: &MeshRanges) {
        self.meshes.give(slot, 1);
        self.meshlets.give(r.meshlets.0, r.meshlets.1);
        self.pages.give(r.pages.0, r.pages.1);
        self.ray_vertices.give(r.ray_vertices.0, r.ray_vertices.1);
        self.ray_triangles
            .give(r.ray_triangles.0, r.ray_triangles.1);
    }
}

/// A mesh of a dynamic scene, as the scene keeps it.
struct LiveMesh {
    record: GpuMesh,
    /// Its finest-level clusters.
    finest: u32,
    ranges: MeshRanges,
}

/// An instance of a dynamic scene, as the scene keeps it.
struct LiveInstance {
    mesh: u32,
    /// What it keeps alive ([`SceneEditor::keep`]).
    kept: Vec<Box<dyn Any + Send>>,
}

/// A mesh an edit brings: its slot, how the scene keeps it, and its structure and that
/// structure's triangles (with rays).
struct AddedMesh {
    slot: u32,
    mesh: LiveMesh,
    blas: Option<(AccelerationStructure, u64)>,
}

/// What the first editor starts from ([`MeshletScene::editor`]): every slot's instance record,
/// and every mesh's record and highest section, as built.
struct EditorSeed {
    instances: Vec<GpuInstance>,
    meshes: Vec<Option<(GpuMesh, u32)>>,
}

/// Something that leaves once no frame reads it.
enum Leaving {
    Instance(u32, LiveInstance),
    Mesh(u32, Box<LiveMesh>),
    /// A top-level structure replaced, an edit's staging buffer.
    Item(Box<dyn Any + Send>),
}

/// An edit taken in by [`MeshletScene::apply`], published at the next frame's start.
pub(super) struct Applied {
    pub(super) staging: Buffer,
    /// Copies from `staging`: (source offset, destination offset, bytes) into the pool, the
    /// page table and the instance table.
    pub(super) pool: Vec<(u64, u64, u64)>,
    pub(super) table: Vec<(u64, u64, u64)>,
    pub(super) instances: Vec<(u64, u64, u64)>,
    tlas: Option<AccelerationStructure>,
    instance_high: u32,
    page_high: u32,
    meshes: Vec<AddedMesh>,
    added: Vec<(u32, LiveInstance)>,
    removed_instances: Vec<u32>,
    removed_meshes: Vec<u32>,
}

/// A dynamic scene's own state (#220).
pub(super) struct SceneDynamic {
    room: Arc<Mutex<Room>>,
    meshes: Vec<Option<LiveMesh>>,
    instances: Vec<Option<LiveInstance>>,
    /// Edits applied since the last frame began.
    staged: Vec<Applied>,
    /// The edits this frame publishes: the copies its "scene/edit" pass makes.
    pub(super) this_frame: Vec<Applied>,
    /// What leaves, from which frame on.
    leaving: Vec<(u64, Leaving)>,
    /// The last frame begun.
    frame: u64,
    /// What the first editor starts from.
    seed: Option<EditorSeed>,
    capacity: DynamicCapacity,
}

/// What an instance of `mesh` adds to the scene's triangles, finest clusters, work bound and
/// clusters over all instances.
fn instance_counts(mesh: &LiveMesh) -> [i64; 4] {
    [
        i64::from(mesh.record.triangle_count),
        i64::from(mesh.finest),
        i64::from(mesh.record.work_bound()),
        i64::from(mesh.record.meshlet_count),
    ]
}

/// What [`MeshletSceneBuilder::build_with`] gives a dynamic scene: its meshes and instances as
/// built, where each lies.
pub(super) struct DynamicSeed<'a> {
    pub capacity: DynamicCapacity,
    pub meshes: &'a [GpuMesh],
    pub mesh_finest: &'a [u32],
    pub mesh_sections: &'a [u32],
    /// Per mesh, its first page; `page_count` pages in all.
    pub mesh_pages: &'a [u32],
    pub page_count: u32,
    /// Per mesh, its rays' vertices and triangles (first, count each); empty without rays.
    pub ray_ranges: &'a [(u32, u32, u32, u32)],
    pub instances: &'a [GpuInstance],
    pub kept: Vec<(usize, Box<dyn Any + Send>)>,
}

impl SceneDynamic {
    pub(super) fn new(seed: DynamicSeed<'_>) -> Self {
        let c = seed.capacity;
        let ray_used = seed
            .ray_ranges
            .last()
            .map_or((0, 0), |r| (r.0 + r.1, r.2 + r.3));
        let room = Room {
            meshes: Ranges::new(c.meshes, seed.meshes.len() as u32),
            instances: Ranges::new(c.instances, seed.instances.len() as u32),
            meshlets: Ranges::new(
                c.meshlets,
                seed.meshes
                    .iter()
                    .map(|m| m.meshlet_offset + m.meshlet_count)
                    .max()
                    .unwrap_or(0),
            ),
            pages: Ranges::new(c.pages, seed.page_count),
            ray_vertices: Ranges::new(c.ray_vertices, ray_used.0),
            ray_triangles: Ranges::new(c.ray_triangles, ray_used.1),
        };
        let meshes = seed
            .meshes
            .iter()
            .enumerate()
            .map(|(m, record)| {
                let first = seed.mesh_pages[m];
                let end = seed
                    .mesh_pages
                    .get(m + 1)
                    .copied()
                    .unwrap_or(seed.page_count);
                let rays = seed.ray_ranges.get(m).copied().unwrap_or_default();
                Some(LiveMesh {
                    record: *record,
                    finest: seed.mesh_finest[m],
                    ranges: MeshRanges {
                        meshlets: (record.meshlet_offset, record.meshlet_count),
                        pages: (first, end - first),
                        ray_vertices: (rays.0, rays.1),
                        ray_triangles: (rays.2, rays.3),
                    },
                })
            })
            .collect();
        let mut instances: Vec<Option<LiveInstance>> = seed
            .instances
            .iter()
            .map(|i| {
                Some(LiveInstance {
                    mesh: i.mesh,
                    kept: Vec::new(),
                })
            })
            .collect();
        for (instance, item) in seed.kept {
            if let Some(Some(live)) = instances.get_mut(instance) {
                live.kept.push(item);
            }
        }
        Self {
            room: Arc::new(Mutex::new(room)),
            meshes,
            instances,
            staged: Vec::new(),
            this_frame: Vec::new(),
            leaving: Vec::new(),
            frame: 0,
            seed: Some(EditorSeed {
                instances: seed.instances.to_vec(),
                meshes: seed
                    .meshes
                    .iter()
                    .zip(seed.mesh_sections)
                    .map(|(m, &s)| Some((*m, s)))
                    .collect(),
            }),
            capacity: c,
        }
    }
}

impl MeshletSceneBuilder {
    /// Fails unless a dynamic scene can be built as this builder stands (#220).
    pub(super) fn check_dynamic(
        &self,
        capacity: DynamicCapacity,
        residency: Residency,
    ) -> Result<()> {
        let fail = |what: String| Err(GpuError::Unsupported(format!("a dynamic scene {what}")));
        if !matches!(residency, Residency::Streamed(_)) {
            return fail("streams its pages".into());
        }
        if self.movers.is_some() || !self.skins.is_empty() {
            return fail("has no movers and no skinned meshes".into());
        }
        let counts = [
            ("meshes", self.meshes.len(), capacity.meshes),
            ("instances", self.instances.len(), capacity.instances),
            ("cluster records", self.meshlets.len(), capacity.meshlets),
            ("pages", self.store.sources.len(), capacity.pages),
        ];
        for (what, count, room) in counts {
            if count > room as usize {
                return fail(format!("of room for {room} {what} built with {count}"));
            }
        }
        Ok(())
    }
}

/// A new mesh of an edit in progress, as the editor made it.
struct NewMesh {
    slot: u32,
    record: GpuMesh,
    finest: u32,
    ranges: MeshRanges,
    /// Its cluster records, its pages numbered as the scene numbers them.
    meshlets: Vec<GpuMeshlet>,
    sources: Vec<PageSource>,
    /// Per page, the pages holding a parent of its clusters (the scene's numbers).
    parents: Vec<Vec<u32>>,
    root_pages: u32,
    cut: Option<Cut>,
}

/// How the rays take a dynamic scene's new mesh ([`SceneEditor::add_mesh`]).
#[derive(Clone, Copy, Debug)]
pub struct MeshRays {
    /// The most triangles of its cut ([`MeshletSceneBuilder::set_ray_budget`]).
    pub budget: u32,
    /// Whether it is ground to the rays ([`MeshletSceneBuilder::set_ray_terrain`]).
    pub terrain: bool,
}

/// The top-level structure's inputs the editor keeps: its own copy of the instance table, the
/// records the instance pass writes, and that pass.
struct EditorTlas {
    instances: Buffer,
    records: Buffer,
    pipeline: Pipeline,
}

/// The worker's side of a dynamic scene (#220; see the module notes): it adds and removes
/// meshes and instances, then [`SceneEditor::finish`] hands the edit to the frames
/// ([`MeshletScene::apply`]).
pub struct SceneEditor {
    device: Arc<Device>,
    room: Arc<Mutex<Room>>,
    meshlets: Arc<Buffer>,
    meshes: Arc<Buffer>,
    rays: Option<RayTables>,
    tlas: Option<EditorTlas>,
    origin: CellPos,
    material_count: u32,
    /// Per instance slot, its record as the edits leave it (a vacant slot's flagged).
    instances: Vec<GpuInstance>,
    /// Per mesh slot, its record and its highest section, while it is in the scene.
    mesh_records: Vec<Option<(GpuMesh, u32)>>,
    /// The edit in progress.
    new_meshes: Vec<NewMesh>,
    added: Vec<u32>,
    kept: Vec<(u32, Box<dyn Any + Send>)>,
    removed_instances: Vec<u32>,
    removed_meshes: Vec<u32>,
}

/// An edit [`SceneEditor::finish`] made, for [`MeshletScene::apply`].
pub struct SceneEdit {
    staging: Buffer,
    pages: PageEdit,
    /// Where the edit's page-table entries go in `staging`.
    table_at: u64,
    instance_copies: Vec<(u64, u64, u64)>,
    tlas: Option<AccelerationStructure>,
    instance_high: u32,
    page_high: u32,
    meshes: Vec<AddedMesh>,
    added: Vec<(u32, LiveInstance)>,
    removed_instances: Vec<u32>,
    removed_meshes: Vec<u32>,
    /// Pages its new meshes' first view wants, loaded with them.
    pub preloaded: u32,
}

impl SceneEdit {
    /// Meshes it brings.
    pub fn meshes_added(&self) -> usize {
        self.meshes.len()
    }
}

/// Fails with "out of room for `what`".
fn no_room<T>(what: &str) -> Result<T> {
    Err(GpuError::Unsupported(format!(
        "the dynamic scene is out of room for {what}"
    )))
}

impl SceneEditor {
    /// Adds `mesh`, its rays cut as `rays` says: its records and its cut go into free ranges of
    /// the scene's tables. Fails when a table has no room for it.
    pub fn add_mesh(&mut self, mesh: &MeshletMesh, rays: MeshRays) -> Result<MeshId> {
        // Its pages as a store of their own, numbered from 0, for its cut.
        let shared = (mesh.page_file.is_none() || !mesh.pages.is_empty())
            .then(|| Arc::new(mesh.pages.clone()));
        let sources = PageSource::of_mesh(mesh, |at| PageSource::Shared {
            bytes: Arc::clone(shared.as_ref().expect("pages in memory")),
            at,
        });
        let cut = match &self.rays {
            Some(_) => {
                let store = PageStore {
                    memory: Arc::default(),
                    sources: sources.clone(),
                };
                let mut cut = raytrace::mesh_cut(&mesh.meshlets, &store, rays.budget, mesh.uvs)?;
                if rays.terrain {
                    cut.shadow_start = raytrace::TERRAIN_SHADOW_START * cut.error;
                }
                Some(cut)
            }
            None => None,
        };
        let (vertices, triangles) = cut.as_ref().map_or((0, 0), |c| {
            (c.positions.len() as u32, c.indices.len() as u32 / 3)
        });
        let sizes = [
            mesh.meshlets.len() as u32,
            mesh.page_count,
            vertices,
            triangles,
        ];
        let taken = self.room.lock().expect("the room").take_mesh(sizes);
        let (slot, ranges) = match taken {
            Ok(taken) => taken,
            Err(what) => return no_room(what),
        };
        let page_base = ranges.pages.0;
        let (record, finest, sections) = mesh_record(mesh, ranges.meshlets.0);
        let parents = ResidentSet::parents_of(mesh.page_count as usize, &mesh.meshlets)
            .into_iter()
            .map(|list| list.into_iter().map(|p| p + page_base).collect())
            .collect();
        self.new_meshes.push(NewMesh {
            slot,
            record,
            finest,
            ranges,
            meshlets: rebased(&mesh.meshlets, page_base).collect(),
            sources,
            parents,
            root_pages: mesh.root_pages,
            cut,
        });
        if self.mesh_records.len() <= slot as usize {
            self.mesh_records.resize(slot as usize + 1, None);
        }
        self.mesh_records[slot as usize] = Some((record, sections));
        Ok(MeshId(slot))
    }

    /// Adds an instance of `mesh` standing at `position` in the world, turned by `rotation` and
    /// scaled by `scale`, in `material` ([`MeshletSceneBuilder::add_instance_at`]). Returns its
    /// slot.
    pub fn add_instance_at(
        &mut self,
        mesh: MeshId,
        position: CellPos,
        rotation: Quat,
        scale: f32,
        material: MaterialId,
    ) -> Result<u32> {
        let Some((info, sections)) = self.mesh_records.get(mesh.0 as usize).copied().flatten()
        else {
            return Err(GpuError::Unsupported(format!(
                "mesh {} is not in the scene",
                mesh.0
            )));
        };
        if material.0 + sections >= self.material_count {
            return Err(GpuError::Unsupported(format!(
                "material {} and {sections} section rows after it, of {} rows",
                material.0, self.material_count
            )));
        }
        let Some(slot) = self.room.lock().expect("the room").instances.take(1) else {
            return no_room("instances");
        };
        let position = position.normalized();
        let center = rotation * (Vec3::from(info.center) * scale) + position.local;
        // Slots past the table's end are handed out in order, so none is skipped; were one, its
        // record would be written vacant, as the culls and the structure read every slot.
        for gap in self.instances.len() as u32..slot {
            self.removed_instances.push(gap);
        }
        let record = GpuInstance {
            cell: position.cell.to_array(),
            mesh: mesh.0,
            local: position.local.to_array(),
            scale,
            rotation: rotation.to_array(),
            center: center.to_array(),
            radius: info.radius * scale,
            id: slot,
            material: material.0,
            flags: 0,
            texture: 0,
        };
        if self.instances.len() <= slot as usize {
            self.instances.resize(slot as usize + 1, vacant());
        }
        self.instances[slot as usize] = record;
        self.added.push(slot);
        Ok(slot)
    }

    /// Gives instance `instance` a texture of its own, by its sampled index
    /// ([`MeshletSceneBuilder::set_instance_texture`]).
    pub fn set_instance_texture(&mut self, instance: u32, sampled: u32) {
        self.instances[instance as usize].texture = sampled + 1;
    }

    /// Has the scene keep `item` alive until instance `instance` leaves it.
    pub fn keep(&mut self, instance: u32, item: impl Any + Send) {
        self.kept.push((instance, Box::new(item)));
    }

    /// Removes instance `instance`: its slot is vacant from the frame the edit is published.
    pub fn remove_instance(&mut self, instance: u32) {
        self.instances[instance as usize].flags = INSTANCE_VACANT;
        self.removed_instances.push(instance);
    }

    /// Removes mesh `mesh`, whose instances are removed in the same edit or before.
    pub fn remove_mesh(&mut self, mesh: MeshId) {
        self.mesh_records[mesh.0 as usize] = None;
        self.removed_meshes.push(mesh.0);
    }

    /// Finishes the edit: the new meshes' tables written and their structures built, their root
    /// pages and the pages `view` wants of them read, the next top-level structure built. The
    /// frames take it in with [`MeshletScene::apply`].
    pub fn finish(&mut self, view: Option<&StartView>) -> Result<SceneEdit> {
        let device = Arc::clone(&self.device);
        let new_meshes = std::mem::take(&mut self.new_meshes);
        // Their structures, in one submission.
        let mut blases: Vec<Option<AccelerationStructure>> = Vec::new();
        if self.rays.is_some() && !new_meshes.is_empty() {
            let triangles: Vec<BlasTriangles<'_>> = new_meshes
                .iter()
                .map(|m| {
                    let cut = m.cut.as_ref().expect("a cut with rays");
                    BlasTriangles {
                        positions: &cut.positions,
                        indices: &cut.indices,
                    }
                })
                .collect();
            blases.extend(
                device
                    .build_blases(&triangles, "mesh BLAS")?
                    .into_iter()
                    .map(Some),
            );
        }
        blases.resize_with(new_meshes.len(), || None);
        // Their tables, where no frame reads.
        let meshlet_size = std::mem::size_of::<GpuMeshlet>() as u64;
        let mesh_size = std::mem::size_of::<GpuMesh>() as u64;
        let mut writes: Vec<(&Buffer, u64, Cow<'_, [u8]>)> = Vec::new();
        for (m, blas) in new_meshes.iter().zip(&blases) {
            writes.push((
                &*self.meshlets,
                u64::from(m.ranges.meshlets.0) * meshlet_size,
                Cow::Borrowed(bytemuck::cast_slice(&m.meshlets)),
            ));
            writes.push((
                &*self.meshes,
                u64::from(m.slot) * mesh_size,
                Cow::Borrowed(bytemuck::bytes_of(&m.record)),
            ));
            if let (Some(tables), Some(cut), Some(blas)) = (&self.rays, &m.cut, blas) {
                writes.extend(raytrace::cut_writes(
                    tables,
                    m.slot,
                    cut,
                    m.ranges.ray_vertices.0,
                    m.ranges.ray_triangles.0,
                    blas.address(),
                ));
            }
        }
        let list: Vec<(&Buffer, u64, &[u8])> = writes
            .iter()
            .map(|(b, at, bytes)| (*b, *at, bytes.as_ref()))
            .collect();
        device.write_buffers_staged(&list)?;

        // The pages: each mesh's roots, then those its first view wants.
        let mut pages = PageEdit::default();
        // Read roots first, then the first view's pages: the staging holds them in that order.
        let mut read_roots: Vec<&PageSource> = Vec::new();
        let mut read_preload: Vec<&PageSource> = Vec::new();
        let mut preload: Vec<(u32, f32)> = Vec::new();
        for m in &new_meshes {
            let first = m.ranges.pages.0;
            for p in 0..m.root_pages {
                pages.roots.push((first + p, 0));
                read_roots.push(&m.sources[p as usize]);
            }
            if let Some(view) = view {
                let instances: Vec<&GpuInstance> = self
                    .added
                    .iter()
                    .map(|&i| &self.instances[i as usize])
                    .filter(|i| i.mesh == m.slot)
                    .collect();
                let mut need = vec![0.0_f32; m.ranges.pages.1 as usize];
                mesh_start_needs(view, &m.record, &m.meshlets, &instances, |page, px| {
                    let p = (page - first) as usize;
                    need[p] = need[p].max(px);
                });
                let local: Vec<Vec<u32>> = m
                    .parents
                    .iter()
                    .map(|list| list.iter().map(|&p| p - first).collect())
                    .collect();
                for (p, n) in preload_order(&need, &local, m.root_pages) {
                    preload.push((first + p, n));
                    read_preload.push(&m.sources[p as usize]);
                }
            }
        }
        let bytes = read_sources(read_roots.into_iter().chain(read_preload), &[])?;
        let roots = pages.roots.len();
        for (k, root) in pages.roots.iter_mut().enumerate() {
            root.1 = (k * PAGE_SIZE) as u64;
        }
        pages.preload = preload
            .iter()
            .enumerate()
            .map(|(k, &(page, need))| (page, need, ((roots + k) * PAGE_SIZE) as u64))
            .collect();
        // The instances' records, after the pages; then room for the page-table entries.
        let instance_size = std::mem::size_of::<GpuInstance>() as u64;
        let changed: Vec<u32> = self
            .added
            .iter()
            .chain(&self.removed_instances)
            .copied()
            .collect();
        let records_at = bytes.len() as u64;
        let table_at = records_at + changed.len() as u64 * instance_size;
        let new_pages: u64 = new_meshes.iter().map(|m| u64::from(m.ranges.pages.1)).sum();
        // An entry per new page, and one per page evicted for those loaded.
        let entries = new_pages + (roots + preload.len()) as u64;
        let staging = device.create_buffer(BufferDesc {
            size: (table_at + entries * 4).max(4),
            usage: vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::CpuToGpu,
            category: MemoryCategory::Transfer,
            name: "scene edit staging",
        })?;
        staging.write(0, &bytes);
        let mut instance_copies = Vec::with_capacity(changed.len());
        for (k, &slot) in changed.iter().enumerate() {
            let at = records_at + k as u64 * instance_size;
            staging.write(at, std::slice::from_ref(&self.instances[slot as usize]));
            instance_copies.push((at, u64::from(slot) * instance_size, instance_size));
        }
        // The top-level structure over every slot up to the highest, vacant ones masked out.
        let instance_high = self.instances.len() as u32;
        let tlas = match &self.tlas {
            Some(t) => {
                let instance_writes: Vec<(&Buffer, u64, &[u8])> = changed
                    .iter()
                    .map(|&slot| {
                        (
                            &t.instances,
                            u64::from(slot) * instance_size,
                            bytemuck::bytes_of(&self.instances[slot as usize]),
                        )
                    })
                    .collect();
                device.write_buffers_staged(&instance_writes)?;
                let tables = self.rays.as_ref().expect("rays with a structure");
                let push = raytrace::tlas_push(
                    t.instances.address(),
                    tables.blas_addresses.address(),
                    t.records.address(),
                    instance_high,
                    self.origin,
                );
                device.execute_compute_once_on(QueueKind::Compute, |commands| {
                    commands.bind_pipeline(&t.pipeline);
                    commands.push_constants(&t.pipeline, &push);
                    commands.dispatch(instance_high.div_ceil(64), 1, 1);
                })?;
                Some(device.build_tlas(t.records.address(), instance_high, "scene TLAS")?)
            }
            None => None,
        };
        pages.meshes = Vec::with_capacity(new_meshes.len());
        let mut meshes = Vec::with_capacity(new_meshes.len());
        for (m, blas) in new_meshes.into_iter().zip(blases) {
            pages.meshes.push((m.ranges.pages.0, m.sources, m.parents));
            let triangles = m.cut.as_ref().map_or(0, |c| c.indices.len() as u64 / 3);
            meshes.push(AddedMesh {
                slot: m.slot,
                mesh: LiveMesh {
                    record: m.record,
                    finest: m.finest,
                    ranges: m.ranges,
                },
                blas: blas.map(|b| (b, triangles)),
            });
        }
        let mut kept = std::mem::take(&mut self.kept);
        let added = std::mem::take(&mut self.added)
            .into_iter()
            .map(|slot| {
                let mut live = LiveInstance {
                    mesh: self.instances[slot as usize].mesh,
                    kept: Vec::new(),
                };
                let mut k = 0;
                while k < kept.len() {
                    if kept[k].0 == slot {
                        live.kept.push(kept.swap_remove(k).1);
                    } else {
                        k += 1;
                    }
                }
                (slot, live)
            })
            .collect();
        let page_high = self.room.lock().expect("the room").pages.high;
        Ok(SceneEdit {
            staging,
            preloaded: preload.len() as u32,
            pages,
            table_at,
            instance_copies,
            tlas,
            instance_high,
            page_high,
            meshes,
            added,
            removed_instances: std::mem::take(&mut self.removed_instances),
            removed_meshes: std::mem::take(&mut self.removed_meshes),
        })
    }
}

/// A vacant slot's record.
fn vacant() -> GpuInstance {
    GpuInstance {
        flags: INSTANCE_VACANT,
        ..GpuInstance::zeroed()
    }
}

/// The pages of one mesh its first view wants (`need`, per page of the mesh, numbered from 0),
/// with every page holding a parent of their clusters: the neediest first, the shallower among
/// equals, a page after all its parents (its first `roots` pages, its roots, pinned already).
fn preload_order(need: &[f32], parents: &[Vec<u32>], roots: u32) -> Vec<(u32, f32)> {
    let mut need = need.to_vec();
    let mut lend: Vec<u32> = (0..need.len() as u32)
        .filter(|&p| need[p as usize] > 0.0)
        .collect();
    while let Some(page) = lend.pop() {
        let value = need[page as usize];
        for &parent in &parents[page as usize] {
            if need[parent as usize] < value {
                need[parent as usize] = value;
                lend.push(parent);
            }
        }
    }
    let depth = streaming::page_depths(parents);
    let mut taken: Vec<bool> = (0..need.len() as u32).map(|p| p < roots).collect();
    let mut wanted: Vec<u32> = (0..need.len() as u32)
        .filter(|&p| need[p as usize] > 0.0 && !taken[p as usize])
        .collect();
    wanted.sort_by(|&a, &b| {
        let (a, b) = (a as usize, b as usize);
        need[b]
            .total_cmp(&need[a])
            .then(depth[a].cmp(&depth[b]))
            .then(a.cmp(&b))
    });
    let mut order = Vec::new();
    for page in wanted {
        if parents[page as usize].iter().all(|&p| taken[p as usize]) {
            taken[page as usize] = true;
            order.push((page, need[page as usize]));
        }
    }
    order
}

impl MeshletScene {
    /// The worker's side of this scene (#220, [`SceneEditor`]), once: a scene built with
    /// [`MeshletSceneBuilder::reserve_dynamic`] has one; `None` for others, or after the first
    /// call.
    pub fn editor(
        &mut self,
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
    ) -> Result<Option<SceneEditor>> {
        let Some(dynamic) = self.dynamic.as_mut() else {
            return Ok(None);
        };
        let Some(EditorSeed {
            instances,
            meshes: mesh_records,
        }) = dynamic.seed.take()
        else {
            return Ok(None);
        };
        let capacity = dynamic.capacity.instances;
        let rays = self.rays.as_ref().and_then(SceneRays::tables);
        let tlas = match &rays {
            Some(_) => {
                let module = device.create_shader_module(
                    &shaders.compile(
                        "meshlet.slang",
                        "tlas_instances_main",
                        ShaderStage::Compute,
                    )?,
                    "edit TLAS instances",
                )?;
                let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                    shader: (module, "tlas_instances_main"),
                    push_constant_bytes: raytrace::TLAS_PUSH_BYTES,
                    name: "edit TLAS instances",
                });
                device.destroy_shader_module(module);
                let instance_table = device.create_buffer(BufferDesc {
                    size: u64::from(capacity.max(1)) * std::mem::size_of::<GpuInstance>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER
                        | vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuOnly,
                    category: MemoryCategory::Geometry,
                    name: "edit TLAS instance table",
                })?;
                device.write_buffer_staged(&instance_table, 0, bytemuck::cast_slice(&instances))?;
                Some(EditorTlas {
                    instances: instance_table,
                    records: device.create_buffer(BufferDesc {
                        size: u64::from(capacity.max(1)) * 64,
                        usage: vk::BufferUsageFlags::STORAGE_BUFFER
                            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                        location: MemoryLocation::GpuOnly,
                        category: MemoryCategory::Transfer,
                        name: "edit TLAS instance records",
                    })?,
                    pipeline: pipeline?,
                })
            }
            None => None,
        };
        Ok(Some(SceneEditor {
            device: Arc::clone(device),
            room: Arc::clone(&dynamic.room),
            meshlets: Arc::clone(&self.meshlets),
            meshes: Arc::clone(&self.meshes),
            rays,
            tlas,
            origin: self.origin,
            material_count: self.material_count,
            instances,
            mesh_records,
            new_meshes: Vec::new(),
            added: Vec::new(),
            kept: Vec::new(),
            removed_instances: Vec::new(),
            removed_meshes: Vec::new(),
        }))
    }

    /// Takes in an edit its editor finished (#220): its pages join the residency now, and the
    /// next frame begun ([`MeshletRenderer::begin_frame`]) publishes it: its copies, its
    /// instances and its top-level structure. Fails, changing nothing, when the pool cannot
    /// hold the new meshes' roots: the editor's view of the scene is then wrong, and the scene
    /// should be built anew.
    pub fn apply(&mut self, edit: SceneEdit) -> Result<()> {
        let (Some(dynamic), Some(streamer)) = (self.dynamic.as_mut(), self.streamer.as_mut())
        else {
            return Err(GpuError::Unsupported(
                "an edit of a scene built whole".into(),
            ));
        };
        let plan = streamer
            .apply(edit.pages, dynamic.frame + 1)
            .map_err(GpuError::Unsupported)?;
        // The page-table entries, in runs of consecutive pages.
        let mut table = Vec::new();
        let mut at = edit.table_at;
        let mut k = 0;
        while k < plan.table.len() {
            let mut end = k + 1;
            while end < plan.table.len() && plan.table[end].0 == plan.table[end - 1].0 + 1 {
                end += 1;
            }
            let values: Vec<u32> = plan.table[k..end].iter().map(|&(_, v)| v).collect();
            edit.staging.write(at, &values);
            let bytes = 4 * values.len() as u64;
            table.push((at, u64::from(plan.table[k].0) * 4, bytes));
            at += bytes;
            k = end;
        }
        debug_assert!(at <= edit.staging.size());
        dynamic.staged.push(Applied {
            staging: edit.staging,
            pool: plan.pages,
            table,
            instances: edit.instance_copies,
            tlas: edit.tlas,
            instance_high: edit.instance_high,
            page_high: edit.page_high,
            meshes: edit.meshes,
            added: edit.added,
            removed_instances: edit.removed_instances,
            removed_meshes: edit.removed_meshes,
        });
        Ok(())
    }

    /// The start of frame `frame` for a dynamic scene: the edits applied since the last frame
    /// are published, and what left `FRAMES_IN_FLIGHT + 1` frames ago is freed.
    pub(super) fn begin_dynamic_frame(&mut self, frame: u64) {
        let Some(dynamic) = self.dynamic.as_mut() else {
            return;
        };
        dynamic.frame = frame;
        // Last frame's copies have been recorded: their staging leaves with that frame.
        for applied in std::mem::take(&mut dynamic.this_frame) {
            dynamic.leaving.push((
                frame + FRAMES_IN_FLIGHT as u64,
                Leaving::Item(Box::new(applied.staging)),
            ));
        }
        let gone = frame + FRAMES_IN_FLIGHT as u64 + 1;
        // The instances' counts: triangles, finest clusters, work bound, clusters.
        let mut counts = [0_i64; 4];
        let mut count = |mesh: &LiveMesh, sign: i64| {
            for (c, v) in counts.iter_mut().zip(instance_counts(mesh)) {
                *c += sign * v;
            }
        };
        for mut applied in std::mem::take(&mut dynamic.staged) {
            self.instance_count = self.instance_count.max(applied.instance_high);
            self.page_count = self.page_count.max(applied.page_high);
            for AddedMesh { slot, mesh, blas } in std::mem::take(&mut applied.meshes) {
                if let (Some(rays), Some((blas, triangles))) = (self.rays.as_mut(), blas) {
                    rays.set_blas(slot, blas, triangles);
                }
                if dynamic.meshes.len() <= slot as usize {
                    dynamic.meshes.resize_with(slot as usize + 1, || None);
                }
                dynamic.meshes[slot as usize] = Some(mesh);
            }
            for (slot, instance) in std::mem::take(&mut applied.added) {
                let mesh = dynamic.meshes[instance.mesh as usize]
                    .as_ref()
                    .expect("an instance's mesh is in the scene");
                count(mesh, 1);
                if dynamic.instances.len() <= slot as usize {
                    dynamic.instances.resize_with(slot as usize + 1, || None);
                }
                dynamic.instances[slot as usize] = Some(instance);
            }
            for slot in std::mem::take(&mut applied.removed_instances) {
                if let Some(instance) = dynamic
                    .instances
                    .get_mut(slot as usize)
                    .and_then(Option::take)
                {
                    if let Some(mesh) = dynamic.meshes[instance.mesh as usize].as_ref() {
                        count(mesh, -1);
                    }
                    dynamic
                        .leaving
                        .push((gone, Leaving::Instance(slot, instance)));
                }
            }
            for slot in std::mem::take(&mut applied.removed_meshes) {
                if let Some(mesh) = dynamic.meshes.get_mut(slot as usize).and_then(Option::take) {
                    dynamic
                        .leaving
                        .push((gone, Leaving::Mesh(slot, Box::new(mesh))));
                }
            }
            if let (Some(rays), Some(tlas)) = (self.rays.as_mut(), applied.tlas.take())
                && let Some(old) = rays.replace_tlas(tlas)
            {
                dynamic.leaving.push((
                    frame + FRAMES_IN_FLIGHT as u64,
                    Leaving::Item(Box::new(old)),
                ));
            }
            dynamic.this_frame.push(applied);
        }
        let add = |total: &mut u64, delta: i64| *total = total.saturating_add_signed(delta);
        add(&mut self.total_triangles, counts[0]);
        add(&mut self.finest_clusters, counts[1]);
        add(&mut self.work_bound, counts[2]);
        add(&mut self.instance_meshlets, counts[3]);
        self.max_meshlets = dynamic
            .meshes
            .iter()
            .flatten()
            .map(|m| m.record.meshlet_count)
            .max()
            .unwrap_or(0);
        // What no frame reads any more.
        let mut k = 0;
        while k < dynamic.leaving.len() {
            if dynamic.leaving[k].0 > frame {
                k += 1;
                continue;
            }
            match dynamic.leaving.swap_remove(k).1 {
                Leaving::Instance(slot, instance) => {
                    drop(instance);
                    dynamic
                        .room
                        .lock()
                        .expect("the room")
                        .instances
                        .give(slot, 1);
                }
                Leaving::Mesh(slot, mesh) => {
                    let (first, count) = mesh.ranges.pages;
                    if let Some(streamer) = self.streamer.as_mut() {
                        streamer.release(first..first + count);
                    }
                    if let Some(rays) = self.rays.as_mut() {
                        drop(rays.take_blas(slot));
                    }
                    dynamic
                        .room
                        .lock()
                        .expect("the room")
                        .give_mesh(slot, &mesh.ranges);
                }
                Leaving::Item(item) => drop(item),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_taken_first_fit_and_given_back_whole() {
        let mut r = Ranges::new(10, 2);
        assert_eq!(r.take(3), Some(2));
        assert_eq!(r.take(3), Some(5));
        assert_eq!(r.take(3), None, "two entries left");
        assert_eq!(r.high, 8);
        // Given back between two taken ranges, then beside the free end: one range again.
        r.give(2, 3);
        assert_eq!(r.free, vec![(2, 3), (8, 2)]);
        assert_eq!(r.take(2), Some(2), "the first that fits");
        r.give(5, 3);
        assert_eq!(r.free, vec![(4, 6)], "merged both ways");
        r.give(2, 2);
        r.give(0, 2);
        assert_eq!(r.free, vec![(0, 10)]);
        assert_eq!(r.take(0), Some(0), "nothing taken for nothing");
        assert_eq!(r.high, 8, "the highest entry ever handed out");
    }

    #[test]
    fn a_mesh_that_does_not_fit_takes_nothing() {
        let mut room = Room {
            meshes: Ranges::new(4, 0),
            instances: Ranges::new(4, 0),
            meshlets: Ranges::new(100, 0),
            pages: Ranges::new(10, 0),
            ray_vertices: Ranges::new(100, 0),
            ray_triangles: Ranges::new(10, 0),
        };
        let (slot, ranges) = room.take_mesh([40, 4, 30, 8]).unwrap();
        assert_eq!((slot, ranges.pages), (0, (0, 4)));
        // Too many triangles: the slot and the ranges taken before go back.
        assert_eq!(
            room.take_mesh([40, 4, 30, 8]).unwrap_err(),
            "rays' triangles"
        );
        assert_eq!(room.meshes.free, vec![(1, 3)]);
        assert_eq!(room.pages.free, vec![(4, 6)]);
        room.give_mesh(slot, &ranges);
        assert_eq!(room.meshlets.free, vec![(0, 100)]);
    }

    #[test]
    fn a_new_mesh_loads_its_wanted_pages_after_their_parents() {
        // Root 0 over 1 and 2; 3 under both.
        let parents = vec![vec![], vec![0], vec![0], vec![1, 2]];
        let need = [0.0, 0.0, 1.0, 3.0];
        // Page 3 lends its need to 1 and 2: 1 and 2 first (the shallower), then 3.
        assert_eq!(
            preload_order(&need, &parents, 1),
            vec![(1, 3.0), (2, 3.0), (3, 3.0)]
        );
        assert!(preload_order(&[0.0; 4], &parents, 1).is_empty());
    }
}

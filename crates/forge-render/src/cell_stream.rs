//! Streamed cells (#338 step 4, D-037's plan, D-077's streamer): a world's cells added and removed
//! around the viewer as it moves, from a game's [`CellSource`].
//!
//! - **The worker** holds the dynamic scene's editor ([`SceneEditor`]), the game's source and the
//!   residency (`forge_world::Residency`). Given the viewer's position, it makes the plan
//!   (`forge_world::StreamPlan`) and takes its changes in batches: the unloads first, then the
//!   loads, nearest and finest first, the cells it faces before those behind it
//!   ([`CellStreamer::view_facing`], #345), up to [`CellStreamConfig::cells_per_edit`] cells an
//!   edit, since each edit builds the editor's top-level structure anew off the frames.
//! - **The frames** take each finished edit and its cells' sets in one update
//!   ([`CellStreamer::update`]): the edit applied ([`MeshletScene::apply`]) and the sets added
//!   or removed ([`CellSystems`]), so a cell comes and goes on the frame the edit is published.
//! - **The far cells' proxies** (D-077): with the policy's `proxy_rings`, a cell past its
//!   content's reach shows its proxy ([`CellSource::load_proxy`]), typically one mesh merged from
//!   its own (`forge_geom::proxy`). A cell swapping between the two has the one taken out and the
//!   other put in by the same edit: never both on a frame, never neither.
//! - **Meshes several cells share** ([`SharedMeshes`]): a source adds one with the first cell
//!   that uses it and takes it out with the last, told when a cell leaves
//!   ([`CellSource::unload`]), rather than keeping every mesh resident from the start.
//! - **Not here:** a cell's own state, which the game keeps and gives back when the cell loads
//!   again.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use forge_gpu::{Device, GpuError, Result};
use forge_world::{CellId, Detail, Partition, Residency, StreamPlan, StreamPolicy};
use glam::DVec3;

use crate::cell_record::{CellRecord, CellSets, CellSystems};
use crate::meshlet::{MeshId, MeshletScene, SceneEdit, SceneEditor};

/// What a cell added to the scene, and its sets: [`CellSource::load`]'s answer.
#[derive(Default)]
pub struct LoadedCell {
    /// The instances it added, which leave when it unloads.
    pub instances: Vec<u32>,
    /// The meshes it added, which leave after its instances.
    pub meshes: Vec<MeshId>,
    /// Its rooms, panes and lamps.
    pub sets: CellSets,
}

/// A game's cells, made on the streamer's worker.
pub trait CellSource: Send + 'static {
    /// Adds cell `cell`'s meshes and instances through `editor` and says what it added. A cell
    /// that fails is left empty (the error logged) until it unloads.
    fn load(&mut self, cell: CellId, editor: &mut SceneEditor) -> Result<LoadedCell>;

    /// Adds cell `cell`'s proxy (D-077), what stands in for it past its content's reach: one mesh
    /// merged from the cell's own seen from afar (`forge_geom::proxy::merge`), cooked once and
    /// kept. A source without proxies adds nothing (the default), and the cell shows nothing
    /// there.
    fn load_proxy(&mut self, cell: CellId, editor: &mut SceneEditor) -> Result<LoadedCell> {
        let _ = (cell, editor);
        Ok(LoadedCell::default())
    }

    /// Cell `cell` has left, at either detail: its instances and its own meshes are out. A source
    /// sharing meshes between cells releases the cell's ([`SharedMeshes::release`]) and takes out
    /// through `editor` those no cell uses any more. Nothing by default.
    fn unload(&mut self, cell: CellId, editor: &mut SceneEditor) {
        let _ = (cell, editor);
    }
}

/// Meshes several cells share, counted by the source's key (#338): one is added with the first
/// cell that asks for it ([`SharedMeshes::acquire`]) and taken out with the last that leaves
/// ([`SharedMeshes::release`], from [`CellSource::unload`]). A world then keeps resident the meshes
/// of the cells in, not every mesh it has. `M` is the mesh's handle, a [`MeshId`].
#[derive(Debug)]
pub struct SharedMeshes<M = MeshId> {
    /// Per key, the mesh and how many cells hold it.
    meshes: HashMap<u64, (M, u32)>,
    /// Per cell, the keys it holds (a key once per cell however often it asked).
    by_cell: HashMap<CellId, Vec<u64>>,
}

impl<M> Default for SharedMeshes<M> {
    fn default() -> Self {
        Self {
            meshes: HashMap::new(),
            by_cell: HashMap::new(),
        }
    }
}

impl<M: Copy> SharedMeshes<M> {
    /// Mesh `key` for cell `cell`: `add` makes and adds it the first time any cell asks for it,
    /// its mesh is returned after that. A cell asking twice holds it once.
    pub fn acquire(
        &mut self,
        cell: CellId,
        key: u64,
        add: impl FnOnce() -> Result<M>,
    ) -> Result<M> {
        if let Some((mesh, count)) = self.meshes.get_mut(&key) {
            let held = self.by_cell.entry(cell).or_default();
            if !held.contains(&key) {
                held.push(key);
                *count += 1;
            }
            return Ok(*mesh);
        }
        let mesh = add()?;
        self.meshes.insert(key, (mesh, 1));
        self.by_cell.entry(cell).or_default().push(key);
        Ok(mesh)
    }

    /// Cell `cell` gone: each mesh it held counted down, and those no cell holds any more
    /// returned, for the source to take out ([`SceneEditor::remove_mesh`]).
    pub fn release(&mut self, cell: CellId) -> Vec<M> {
        let mut gone = Vec::new();
        for key in self.by_cell.remove(&cell).unwrap_or_default() {
            if let Some((mesh, count)) = self.meshes.get_mut(&key) {
                *count -= 1;
                if *count == 0 {
                    gone.push(*mesh);
                    self.meshes.remove(&key);
                }
            }
        }
        gone
    }

    /// Meshes held.
    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
}

/// How a [`CellStreamer`] streams.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStreamConfig {
    /// The plan's levels and reaches (`forge_world::StreamPolicy`), the proxies' with them.
    pub policy: StreamPolicy,
    /// The most cells an edit brings or takes (a swap counts once each way).
    pub cells_per_edit: usize,
}

/// One edit and the cells it brings or takes.
struct Batch {
    edit: SceneEdit,
    /// The cells it brings, at their detail: a swapped cell is in `unloaded` too, taken out
    /// first.
    loaded: Vec<(CellId, Detail, CellSets)>,
    unloaded: Vec<CellId>,
    /// How many of the loaded swapped from the other detail.
    swapped: u32,
    /// How long the worker took over it.
    took: Duration,
}

/// What [`CellStreamer::update`] took in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellStreamStats {
    /// Edits applied.
    pub edits: u32,
    /// Cells added, at either detail.
    pub loaded: u32,
    /// Cells removed, at either detail.
    pub unloaded: u32,
    /// Cells swapped between their content and their proxy (each also counted loaded and
    /// unloaded).
    pub swapped: u32,
    /// The worker's time over them, seconds.
    pub worker_s: f64,
}

impl std::ops::AddAssign for CellStreamStats {
    fn add_assign(&mut self, o: Self) {
        self.edits += o.edits;
        self.loaded += o.loaded;
        self.unloaded += o.unloaded;
        self.swapped += o.swapped;
        self.worker_s += o.worker_s;
    }
}

/// A world's cells streamed around the viewer (see the module notes).
pub struct CellStreamer {
    viewer: Sender<(u64, DVec3, Option<DVec3>)>,
    /// The positions given so far.
    sent: u64,
    /// The last position whose changes the worker has all made and sent.
    settled: Arc<AtomicU64>,
    batches: Receiver<Result<Batch>>,
    records: HashMap<CellId, (Detail, CellRecord)>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl CellStreamer {
    /// Starts the worker with the scene's `editor`, the game's `source` and the world's
    /// `partition` (its frame the viewer's).
    pub fn new<P, S>(
        editor: SceneEditor,
        source: S,
        partition: P,
        config: CellStreamConfig,
    ) -> Result<Self>
    where
        P: Partition + Send + 'static,
        S: CellSource,
    {
        let (viewer, viewers) = mpsc::channel::<(u64, DVec3, Option<DVec3>)>();
        // Two edits ahead at most: each holds its staging until the frames take it.
        let (done, batches) = mpsc::sync_channel::<Result<Batch>>(2);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let settled = Arc::new(AtomicU64::new(0));
        let settling = Arc::clone(&settled);
        let worker = std::thread::Builder::new()
            .name("cell stream".to_owned())
            .spawn(move || {
                let mut w = Worker {
                    editor,
                    source,
                    partition,
                    config,
                    residency: Residency::new(),
                    live: HashMap::new(),
                    loads: VecDeque::new(),
                    unloads: VecDeque::new(),
                };
                w.run(&viewers, &done, &stopping, &settling);
            })
            .map_err(|e| GpuError::Unsupported(format!("the cell stream's worker: {e}")))?;
        Ok(Self {
            viewer,
            sent: 0,
            settled,
            batches,
            records: HashMap::new(),
            stop,
            worker: Some(worker),
        })
    }

    /// The viewer's position for the next plan (the partition's frame), once a frame; its
    /// number, for [`Self::update_settled`].
    pub fn view(&mut self, at: DVec3) -> u64 {
        self.view_facing(at, None)
    }

    /// As [`Self::view`], with the way the viewer faces (the partition's frame): the cells it
    /// faces load first (#345), and a turn of more than about 15° plans again.
    pub fn view_facing(&mut self, at: DVec3, facing: Option<DVec3>) -> u64 {
        self.sent += 1;
        let _ = self.viewer.send((self.sent, at, facing));
        self.sent
    }

    /// [`Self::update`] until the worker has made, and the frames taken, every change the
    /// position numbered `seq` asks for ([`Self::view`]): a capture's runs, whose cells must not
    /// hang on the worker's timing.
    pub fn update_settled(
        &mut self,
        seq: u64,
        device: &Arc<Device>,
        scene: &mut MeshletScene,
        systems: &mut CellSystems<'_>,
    ) -> Result<CellStreamStats> {
        let mut total = CellStreamStats::default();
        loop {
            // Read first: every edit sent before it is then in the channel.
            let settled = self.settled.load(Ordering::Acquire);
            let took = self.update(
                device,
                scene,
                systems,
                u32::MAX,
                Some(Duration::from_millis(1)),
            )?;
            total += took;
            if settled >= seq && took.edits == 0 {
                return Ok(total);
            }
            if self.worker.as_ref().is_none_or(JoinHandle::is_finished) {
                return Err(GpuError::Unsupported(
                    "the cell stream's worker stopped".into(),
                ));
            }
        }
    }

    /// Takes in the edits the worker finished, at most `max_edits`: each applied with its cells'
    /// sets added or removed through `systems`, in this one update. With `wait`, waits for one
    /// first (a capture's runs, where a frame's cells must not depend on the worker's timing).
    pub fn update(
        &mut self,
        device: &Arc<Device>,
        scene: &mut MeshletScene,
        systems: &mut CellSystems<'_>,
        max_edits: u32,
        wait: Option<Duration>,
    ) -> Result<CellStreamStats> {
        let mut stats = CellStreamStats::default();
        while stats.edits < max_edits {
            let next = match (stats.edits, wait) {
                (0, Some(timeout)) => self.batches.recv_timeout(timeout).ok(),
                _ => self.batches.try_recv().ok(),
            };
            let Some(batch) = next else {
                break;
            };
            let batch = batch?;
            scene.apply(batch.edit)?;
            // Out first: a swapped cell's old record goes before its new one comes.
            for cell in &batch.unloaded {
                if let Some((_, record)) = self.records.remove(cell) {
                    systems.remove(&record);
                }
            }
            for (cell, detail, sets) in &batch.loaded {
                match systems.add(device, sets) {
                    Ok(record) => {
                        self.records.insert(*cell, (*detail, record));
                    }
                    Err(e) => tracing::warn!(?cell, "a streamed cell's sets: {e}"),
                }
            }
            stats.edits += 1;
            stats.loaded += batch.loaded.len() as u32;
            stats.unloaded += batch.unloaded.len() as u32;
            stats.swapped += batch.swapped;
            stats.worker_s += batch.took.as_secs_f64();
        }
        Ok(stats)
    }

    /// The cells in, as the frames have them, at either detail.
    pub fn loaded(&self) -> usize {
        self.records.len()
    }

    /// Of them, those shown by their proxy.
    pub fn proxies(&self) -> usize {
        self.records
            .values()
            .filter(|(detail, _)| *detail == Detail::Proxy)
            .count()
    }
}

impl Drop for CellStreamer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The worker blocked on a full channel wakes as the batches drop.
        while self.batches.try_recv().is_ok() {}
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// What a cell in has added, at which detail.
struct Live {
    detail: Detail,
    instances: Vec<u32>,
    meshes: Vec<MeshId>,
}

/// The worker's side.
struct Worker<P, S> {
    editor: SceneEditor,
    source: S,
    partition: P,
    config: CellStreamConfig,
    residency: Residency,
    /// What each cell in added.
    live: HashMap<CellId, Live>,
    /// The changes not made yet: cells to bring in at a detail (or swap to it), cells to take out.
    loads: VecDeque<(CellId, Detail)>,
    unloads: VecDeque<CellId>,
}

impl<P: Partition, S: CellSource> Worker<P, S> {
    fn run(
        &mut self,
        viewers: &Receiver<(u64, DVec3, Option<DVec3>)>,
        done: &SyncSender<Result<Batch>>,
        stop: &AtomicBool,
        settled: &AtomicU64,
    ) {
        // Where the last plan was made from, and the way it faced.
        let mut viewer: Option<(DVec3, Option<DVec3>)> = None;
        let mut seq = 0;
        while !stop.load(Ordering::Relaxed) {
            // The latest position; wait for one when there is nothing to do.
            let idle = self.loads.is_empty() && self.unloads.is_empty();
            let first = if idle {
                match viewers.recv_timeout(Duration::from_millis(50)) {
                    Ok(given) => Some(given),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                viewers.try_recv().ok()
            };
            if let Some((n, at, facing)) = first.map(|g| viewers.try_iter().last().unwrap_or(g)) {
                seq = n;
                let facing = facing.and_then(DVec3::try_normalize);
                // Moved, or turned by more than about 15°.
                let again = viewer.is_none_or(|(was, faced)| {
                    was != at
                        || match (faced, facing) {
                            (Some(a), Some(b)) => a.dot(b) < 0.966,
                            (a, b) => a.is_some() != b.is_some(),
                        }
                });
                if again {
                    viewer = Some((at, facing));
                    self.plan(at, facing);
                }
            }
            if self.loads.is_empty() && self.unloads.is_empty() {
                settled.store(seq, Ordering::Release);
                continue;
            }
            let batch = self.batch();
            if done.send(batch).is_err() {
                return;
            }
            if self.loads.is_empty() && self.unloads.is_empty() {
                settled.store(seq, Ordering::Release);
            }
        }
    }

    /// The plan around `at` facing `facing`, its changes queued. A cell queued to load and now to
    /// unload is dropped from the loads, and taken out if it is in at all; a cell queued twice
    /// keeps its latest detail, and none when that is the detail it is in at. The loads queued
    /// take the plan's order, the cells faced first.
    fn plan(&mut self, at: DVec3, facing: Option<DVec3>) {
        let plan = StreamPlan::around_facing(&self.partition, at, facing, &self.config.policy);
        let changes = self
            .residency
            .update(&self.partition, at, &plan, &self.config.policy);
        for cell in changes.unload {
            self.loads.retain(|&(c, _)| c != cell);
            if self.live.contains_key(&cell) && !self.unloads.contains(&cell) {
                self.unloads.push_back(cell);
            }
        }
        // The plan's order: nearest and finest first.
        for (cell, detail) in changes.load {
            self.loads.retain(|&(c, _)| c != cell);
            let back = self.live.get(&cell).is_some_and(|l| l.detail == detail);
            if back && !self.unloads.contains(&cell) {
                continue;
            }
            self.loads.push_back((cell, detail));
        }
        let priority: HashMap<CellId, f64> =
            plan.wants.iter().map(|w| (w.cell, w.priority)).collect();
        let rank = |cell: &CellId| priority.get(cell).copied().unwrap_or(f64::INFINITY);
        self.loads
            .make_contiguous()
            .sort_by(|a, b| rank(&a.0).total_cmp(&rank(&b.0)));
    }

    /// Takes out what cell `cell` added, then tells its source it left.
    fn take_out(&mut self, cell: CellId, live: Live) {
        for i in live.instances {
            self.editor.remove_instance(i);
        }
        for m in live.meshes {
            self.editor.remove_mesh(m);
        }
        self.source.unload(cell, &mut self.editor);
    }

    /// The next edit: unloads first, then loads, up to the configured cells. A cell in at the
    /// other detail is taken out in the same edit that brings it at this one.
    fn batch(&mut self) -> Result<Batch> {
        let start = Instant::now();
        let mut unloaded = Vec::new();
        let mut loaded = Vec::new();
        let mut swapped = 0;
        let budget = self.config.cells_per_edit.max(1);
        while unloaded.len() < budget
            && let Some(cell) = self.unloads.pop_front()
        {
            if let Some(live) = self.live.remove(&cell) {
                self.take_out(cell, live);
            }
            unloaded.push(cell);
        }
        while unloaded.len() + loaded.len() < budget
            && let Some((cell, detail)) = self.loads.pop_front()
        {
            if let Some(live) = self.live.remove(&cell) {
                self.take_out(cell, live);
                unloaded.push(cell);
                swapped += 1;
            }
            let made = match detail {
                Detail::Full => self.source.load(cell, &mut self.editor),
                Detail::Proxy => self.source.load_proxy(cell, &mut self.editor),
            };
            match made {
                Ok(cell_in) => {
                    self.live.insert(
                        cell,
                        Live {
                            detail,
                            instances: cell_in.instances,
                            meshes: cell_in.meshes,
                        },
                    );
                    loaded.push((cell, detail, cell_in.sets));
                }
                Err(e) => tracing::warn!(?cell, ?detail, "a streamed cell not made: {e}"),
            }
        }
        let edit = self.editor.finish(None)?;
        Ok(Batch {
            edit,
            loaded,
            unloaded,
            swapped,
            took: start.elapsed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mesh two cells share is added once, with the first, and taken out with the last to
    /// leave; asked again after, it is added again.
    #[test]
    fn a_shared_mesh_comes_with_the_first_cell_and_goes_with_the_last() {
        let (a, b) = (CellId::flat(4, 0, 0), CellId::flat(4, 1, 0));
        let mut shared = SharedMeshes::<u32>::default();
        let added = std::cell::Cell::new(0);
        let add = |id: u32| {
            added.set(added.get() + 1);
            Ok(id)
        };
        assert_eq!(shared.acquire(a, 7, || add(70)).unwrap(), 70);
        assert_eq!(
            shared.acquire(a, 7, || add(71)).unwrap(),
            70,
            "a cell asking twice"
        );
        assert_eq!(shared.acquire(b, 7, || add(72)).unwrap(), 70);
        assert_eq!(shared.acquire(b, 8, || add(80)).unwrap(), 80);
        assert_eq!(added.get(), 2);
        assert_eq!(shared.len(), 2);
        assert_eq!(shared.release(a), Vec::<u32>::new(), "b still holds 7");
        let mut gone = shared.release(b);
        gone.sort_unstable();
        assert_eq!(gone, [70, 80]);
        assert!(shared.is_empty());
        assert_eq!(shared.acquire(a, 7, || add(73)).unwrap(), 73, "added again");
        assert_eq!(added.get(), 3);
        assert_eq!(
            shared.release(CellId::flat(4, 9, 9)),
            Vec::<u32>::new(),
            "a cell that held none"
        );
    }

    /// A mesh that fails to be made is held by no cell.
    #[test]
    fn a_mesh_not_made_is_not_held() {
        let a = CellId::flat(4, 0, 0);
        let mut shared = SharedMeshes::<u32>::default();
        assert!(
            shared
                .acquire(a, 1, || Err(GpuError::Unsupported("no room".into())))
                .is_err()
        );
        assert!(shared.is_empty());
        assert_eq!(shared.release(a), Vec::<u32>::new());
    }
}

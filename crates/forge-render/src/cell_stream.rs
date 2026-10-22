//! Streamed cells (#338 step 4, D-037's plan, D-077's streamer): a world's cells added and removed
//! around the viewer as it moves, from a game's [`CellSource`].
//!
//! - **The worker** holds the dynamic scene's editor ([`SceneEditor`]), the game's source and the
//!   residency (`forge_world::Residency`). Given the viewer's position, it makes the plan
//!   (`forge_world::StreamPlan`) and takes its changes in batches: the unloads first, then the
//!   loads, nearest and finest first, up to [`CellStreamConfig::cells_per_edit`] cells an edit,
//!   since each edit builds the editor's top-level structure anew off the frames.
//! - **The frames** take each finished edit and its cells' sets in one update
//!   ([`CellStreamer::update`]): the edit applied ([`MeshletScene::apply`]) and the sets added
//!   or removed ([`CellSystems`]), so a cell comes and goes on the frame the edit is published.
//! - **Not here:** the far cells' look (D-077's proxies, the owner's to decide), and a cell's own
//!   state, which the game keeps and gives back when the cell loads again.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use forge_gpu::{Device, GpuError, Result};
use forge_world::{CellId, Partition, Residency, StreamPlan, StreamPolicy};
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
}

/// How a [`CellStreamer`] streams.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellStreamConfig {
    /// The plan's levels and reaches (`forge_world::StreamPolicy`).
    pub policy: StreamPolicy,
    /// The most cells an edit brings or takes.
    pub cells_per_edit: usize,
}

/// One edit and the cells it brings or takes.
struct Batch {
    edit: SceneEdit,
    loaded: Vec<(CellId, CellSets)>,
    unloaded: Vec<CellId>,
    /// How long the worker took over it.
    took: Duration,
}

/// What [`CellStreamer::update`] took in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellStreamStats {
    /// Edits applied.
    pub edits: u32,
    /// Cells added.
    pub loaded: u32,
    /// Cells removed.
    pub unloaded: u32,
    /// The worker's time over them, seconds.
    pub worker_s: f64,
}

impl std::ops::AddAssign for CellStreamStats {
    fn add_assign(&mut self, o: Self) {
        self.edits += o.edits;
        self.loaded += o.loaded;
        self.unloaded += o.unloaded;
        self.worker_s += o.worker_s;
    }
}

/// A world's cells streamed around the viewer (see the module notes).
pub struct CellStreamer {
    viewer: Sender<(u64, DVec3)>,
    /// The positions given so far.
    sent: u64,
    /// The last position whose changes the worker has all made and sent.
    settled: Arc<AtomicU64>,
    batches: Receiver<Result<Batch>>,
    records: HashMap<CellId, CellRecord>,
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
        let (viewer, viewers) = mpsc::channel::<(u64, DVec3)>();
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
        self.sent += 1;
        let _ = self.viewer.send((self.sent, at));
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
            for cell in &batch.unloaded {
                if let Some(record) = self.records.remove(cell) {
                    systems.remove(&record);
                }
            }
            for (cell, sets) in &batch.loaded {
                match systems.add(device, sets) {
                    Ok(record) => {
                        self.records.insert(*cell, record);
                    }
                    Err(e) => tracing::warn!(?cell, "a streamed cell's sets: {e}"),
                }
            }
            stats.edits += 1;
            stats.loaded += batch.loaded.len() as u32;
            stats.unloaded += batch.unloaded.len() as u32;
            stats.worker_s += batch.took.as_secs_f64();
        }
        Ok(stats)
    }

    /// The cells in, as the frames have them.
    pub fn loaded(&self) -> usize {
        self.records.len()
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

/// The worker's side.
struct Worker<P, S> {
    editor: SceneEditor,
    source: S,
    partition: P,
    config: CellStreamConfig,
    residency: Residency,
    /// What each cell in added: its instances and meshes.
    live: HashMap<CellId, (Vec<u32>, Vec<MeshId>)>,
    /// The changes not made yet.
    loads: VecDeque<CellId>,
    unloads: VecDeque<CellId>,
}

impl<P: Partition, S: CellSource> Worker<P, S> {
    fn run(
        &mut self,
        viewers: &Receiver<(u64, DVec3)>,
        done: &SyncSender<Result<Batch>>,
        stop: &AtomicBool,
        settled: &AtomicU64,
    ) {
        let mut viewer: Option<DVec3> = None;
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
            if let Some((n, at)) = first.map(|g| viewers.try_iter().last().unwrap_or(g)) {
                seq = n;
                if viewer != Some(at) {
                    viewer = Some(at);
                    self.plan(at);
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

    /// The plan around `at`, its changes queued: a cell queued to load and now to unload is
    /// dropped from both.
    fn plan(&mut self, at: DVec3) {
        let plan = StreamPlan::around(&self.partition, at, &self.config.policy);
        let changes = self
            .residency
            .update(&self.partition, at, &plan, &self.config.policy);
        for cell in changes.unload {
            if let Some(k) = self.loads.iter().position(|&c| c == cell) {
                self.loads.remove(k);
            } else {
                self.unloads.push_back(cell);
            }
        }
        // The plan's order: nearest and finest first.
        self.loads.extend(changes.load);
    }

    /// The next edit: unloads first, then loads, up to the configured cells.
    fn batch(&mut self) -> Result<Batch> {
        let start = Instant::now();
        let mut unloaded = Vec::new();
        let mut loaded = Vec::new();
        let budget = self.config.cells_per_edit.max(1);
        while unloaded.len() < budget
            && let Some(cell) = self.unloads.pop_front()
        {
            if let Some((instances, meshes)) = self.live.remove(&cell) {
                for i in instances {
                    self.editor.remove_instance(i);
                }
                for m in meshes {
                    self.editor.remove_mesh(m);
                }
            }
            unloaded.push(cell);
        }
        while unloaded.len() + loaded.len() < budget
            && let Some(cell) = self.loads.pop_front()
        {
            match self.source.load(cell, &mut self.editor) {
                Ok(cell_in) => {
                    self.live.insert(cell, (cell_in.instances, cell_in.meshes));
                    loaded.push((cell, cell_in.sets));
                }
                Err(e) => tracing::warn!(?cell, "a streamed cell not made: {e}"),
            }
        }
        let edit = self.editor.finish(None)?;
        Ok(Batch {
            edit,
            loaded,
            unloaded,
            took: start.elapsed(),
        })
    }
}

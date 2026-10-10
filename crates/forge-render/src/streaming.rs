//! Cluster-page streaming (issue #36, D-025).
//!
//! A streamed scene keeps its cluster pages (`forge_geom::page`) in their cache files (or in
//! memory) and gives the GPU a pool of page slots smaller than the scene. Every frame:
//! 1. **Needs.** The cluster culls write, per page, the largest projected error in pixels the
//!    page takes away: a cluster too coarse for the threshold wants its children's page, a
//!    drawn one keeps its own (`want_page` in `meshlet.slang`). The frame copies the array
//!    back; the streamer reads it when that frame slot comes round again.
//! 2. **Uploads.** Up to `upload_pages` pages read since then are copied into the pool by a
//!    `streaming/upload` pass before the culls: into a free slot, or into the slot of the
//!    resident page with the lowest need (the least recently wanted among equals, D-018),
//!    and only when that need is lower than the new page's. The page table follows in the
//!    same pass.
//! 3. **Requests.** Absent pages with a need, whose parent pages are all resident, go to the
//!    I/O thread, the neediest first, at most `reads_in_flight` at a time.
//!
//! Residency stays closed upwards (Nanite's dependencies): a page loads only when every page
//! holding a parent of its clusters is resident, and only pages with no resident children
//! leave. A page holds a dozen groups whose parents lie in several pages, some of which the
//! cut may not want for themselves: a wanted page therefore lends its need to all its
//! ancestors, which are then requested first (the coarsest missing ones) and kept. The
//! roots' pages are loaded at start and never leave. The LOD cut refines a cluster only
//! when its children's page is resident, so whatever is missing, the image loses detail,
//! never a piece.
//!
//! A scene given a start view ([`StartView`], #121) also loads, before its first frame, the
//! pages that view's cut wants, worked out on the CPU with the culls' metric and a margin
//! ([`crate::MeshletScene::load_start_view`]). Its first frames then draw what a resident scene
//! draws instead of refining over the first second, and a fixed view streams nothing: its
//! frames depend on no read's timing.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use crate::cells::CellPos;
use forge_core::pack::{Codec, par_chunks_mut, unpack};
use forge_geom::{GpuMeshlet, PAGE_NONE, PAGE_SIZE, PackedPage};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, GraphBuffer, MemoryCategory, MemoryLocation,
    Result, vk,
};

/// How a scene's cluster pages are made resident.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Residency {
    /// Every page, uploaded once when the scene is built.
    All,
    /// A pool of page slots filled on demand (see the module notes).
    Streamed(StreamingConfig),
}

/// The budgets of a streamed scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamingConfig {
    /// Page slots on the GPU, the roots' pages included (128 KiB each).
    pub pool_pages: u32,
    /// Pages uploaded per frame at most.
    pub upload_pages: u32,
    /// Page reads queued on the I/O thread at most.
    pub reads_in_flight: u32,
}

impl StreamingConfig {
    /// A pool of `pool_mib` MiB and an upload budget of `upload_mib` MiB per frame.
    pub fn from_mib(pool_mib: u32, upload_mib: u32) -> Self {
        let pages = |mib: u32| ((u64::from(mib) << 20) / PAGE_SIZE as u64).max(1) as u32;
        Self {
            pool_pages: pages(pool_mib),
            upload_pages: pages(upload_mib),
            reads_in_flight: pages(upload_mib) * 4,
        }
    }
}

/// A camera whose cut a streamed scene loads before its first frame
/// ([`crate::MeshletSceneBuilder::set_start_view`], #121; see the module notes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartView {
    /// Where the camera stands.
    pub position: CellPos,
    /// The projection's vertical scale ([`crate::CullCamera::p11`]).
    pub p11: f32,
    /// The near plane, metres.
    pub near: f32,
    /// The drawn image's height in pixels.
    pub viewport_height: u32,
    /// The projected error a drawn cluster may have, in pixels
    /// ([`crate::DrawParams::lod_threshold_px`]).
    pub lod_threshold_px: f32,
}

/// The share of the threshold a start view's cut goes down to: a page loads when its
/// clusters' parents would show more than this share of it, a margin over the difference
/// between the CPU's `f64` and the culls' camera-relative `f32`.
pub(crate) const START_MARGIN: f32 = 0.9;

/// Per page, the longest chain of pages above it (0 for a page that needs none).
pub(crate) fn page_depths(parents: &[Vec<u32>]) -> Vec<u32> {
    const UNSET: u32 = u32::MAX;
    const VISITING: u32 = u32::MAX - 1;
    let mut depth = vec![UNSET; parents.len()];
    let mut stack = Vec::new();
    for start in 0..parents.len() {
        if depth[start] != UNSET {
            continue;
        }
        stack.push(start);
        while let Some(&page) = stack.last() {
            if depth[page] != UNSET && depth[page] != VISITING {
                stack.pop();
                continue;
            }
            depth[page] = VISITING;
            let mut ready = true;
            let mut d = 0;
            for &parent in &parents[page] {
                match depth[parent as usize] {
                    UNSET => {
                        stack.push(parent as usize);
                        ready = false;
                    }
                    // A cycle, which the residency rules would never load: ignored here.
                    VISITING => {}
                    value => d = d.max(value + 1),
                }
            }
            if ready {
                depth[page] = d;
                stack.pop();
            }
        }
    }
    depth
}

/// Where a page's bytes are.
#[derive(Clone, Debug)]
pub(crate) enum PageSource {
    /// At this byte offset of [`PageStore::memory`].
    Memory(usize),
    /// Packed in this file (#215).
    File { file: Arc<Path>, page: PackedPage },
    /// At this byte offset of the pages a dynamic scene's mesh brought in memory (#220).
    Shared { bytes: Arc<Vec<u8>>, at: usize },
    /// No page: a dynamic scene's page number no mesh holds (#220).
    Absent,
}

impl PageSource {
    /// The sources of `mesh`'s pages: in its page file, or in memory, `memory` giving the source
    /// of the page at a byte offset of its pages when they are not in a file.
    pub fn of_mesh(mesh: &forge_geom::MeshletMesh, memory: impl Fn(usize) -> Self) -> Vec<Self> {
        match &mesh.page_file {
            Some(pages) if mesh.pages.is_empty() => {
                let file: Arc<Path> = Arc::from(pages.path.as_path());
                pages
                    .pages
                    .iter()
                    .map(|&page| Self::File {
                        file: Arc::clone(&file),
                        page,
                    })
                    .collect()
            }
            _ => (0..mesh.page_count as usize)
                .map(|p| memory(p * PAGE_SIZE))
                .collect(),
        }
    }
}

/// Every page of a scene and where to read it.
#[derive(Default)]
pub(crate) struct PageStore {
    /// The pages of meshes that keep them in memory, one after the other.
    pub memory: Arc<Vec<u8>>,
    /// Per scene page.
    pub sources: Vec<PageSource>,
}

/// Page `source`'s bytes as they are stored, and how they are packed, `memory` holding the
/// [`PageSource::Memory`] pages; `open` keeps the files open between calls.
fn read_packed<'a>(
    source: &'a PageSource,
    memory: &'a [u8],
    open: &mut HashMap<Arc<Path>, File>,
) -> io::Result<(Codec, Cow<'a, [u8]>)> {
    match source {
        &PageSource::Memory(at) => Ok((Codec::Raw, Cow::Borrowed(&memory[at..at + PAGE_SIZE]))),
        PageSource::Shared { bytes, at } => {
            Ok((Codec::Raw, Cow::Borrowed(&bytes[*at..*at + PAGE_SIZE])))
        }
        PageSource::File { file, page } => {
            let handle = match open.entry(Arc::clone(file)) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(File::open(file)?),
            };
            handle.seek(SeekFrom::Start(page.offset))?;
            let mut packed = vec![0; page.len as usize];
            handle.read_exact(&mut packed)?;
            Ok((page.codec, Cow::Owned(packed)))
        }
        PageSource::Absent => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no page at this number",
        )),
    }
}

/// The bytes of `sources`' pages, one after the other, `memory` holding the
/// [`PageSource::Memory`] pages: read in order, unpacked in parallel (#215).
pub(crate) fn read_sources<'a>(
    sources: impl Iterator<Item = &'a PageSource>,
    memory: &[u8],
) -> io::Result<Vec<u8>> {
    let mut open = HashMap::new();
    let packed = sources
        .map(|source| read_packed(source, memory, &mut open))
        .collect::<io::Result<Vec<_>>>()?;
    let mut bytes = vec![0; packed.len() * PAGE_SIZE];
    let failed = Mutex::new(None);
    par_chunks_mut(&mut bytes, PAGE_SIZE, 8, |i, out| {
        let (codec, page) = &packed[i];
        if let Err(error) = unpack(*codec, page, out) {
            *failed.lock().unwrap() = Some(error);
        }
    });
    match failed.into_inner().unwrap() {
        Some(error) => Err(error),
        None => Ok(bytes),
    }
}

impl PageStore {
    /// The bytes of `pages`, one after the other: read in order, unpacked in parallel (#215).
    pub fn read_pages(&self, pages: impl Iterator<Item = u32>) -> io::Result<Vec<u8>> {
        read_sources(pages.map(|page| &self.sources[page as usize]), &self.memory)
    }
}

/// What a streamed scene did in one frame, for the overlay and the logs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamingStats {
    /// Page slots in the pool.
    pub pool_pages: u32,
    /// Slots holding a page.
    pub resident: u32,
    /// Pages the frame's cut wanted that are not resident.
    pub wanted: u32,
    /// Reads sent to the I/O thread this frame.
    pub requested: u32,
    /// Reads not finished.
    pub reading: u32,
    /// Pages uploaded this frame.
    pub uploaded: u32,
    /// Pages evicted this frame to make room.
    pub evicted: u32,
}

impl StreamingStats {
    /// Bytes uploaded this frame.
    pub fn bytes_uploaded(&self) -> u64 {
        u64::from(self.uploaded) * PAGE_SIZE as u64
    }

    /// One counter line.
    pub fn line(&self) -> String {
        let mib = |pages: u32| f64::from(pages) * PAGE_SIZE as f64 / f64::from(1 << 20);
        format!(
            "streaming: {} of {} pages resident ({:.0} of {:.0} MiB), {} wanted, {} requested, {} reading, {} uploaded ({:.1} MiB), {} evicted",
            self.resident,
            self.pool_pages,
            mib(self.resident),
            mib(self.pool_pages),
            self.wanted,
            self.requested,
            self.reading,
            self.uploaded,
            mib(self.uploaded),
            self.evicted,
        )
    }
}

/// The copies a frame's upload pass records: (staging offset, destination offset, bytes)
/// into the pool, then into the page table.
#[derive(Default)]
pub(crate) struct UploadPlan {
    pub pages: Vec<(u64, u64, u64)>,
    pub table: Vec<(u64, u64, u64)>,
}

/// A page read from its source (or the error that stopped it), with the generation of its
/// number it was asked for under.
type Loaded = (u32, u32, io::Result<Vec<u8>>);

/// A read for the I/O thread: the page, its number's generation, where its bytes are.
type ReadRequest = (u32, u32, PageSource);

/// What [`ResidentSet::place`] decided for a frame's loaded pages.
pub(crate) struct Placement<T> {
    /// Pages to upload: (page, slot, bytes).
    pub placed: Vec<(u32, u32, T)>,
    /// Pages whose slot a placed page takes (their page-table entries become `PAGE_NONE`).
    pub evicted: Vec<u32>,
}

/// Which page sits in which pool slot, and the rules that keep the resident set closed
/// upwards (see the module notes). No GPU and no I/O: [`PageStreamer`] feeds it the needs
/// read back and the pages read, and applies what it decides.
pub(crate) struct ResidentSet {
    /// Per page: its slot, or `PAGE_NONE`.
    slot_of: Vec<u32>,
    /// Per slot: its page, or `PAGE_NONE`.
    page_in_slot: Vec<u32>,
    free_slots: Vec<u32>,
    pinned: Vec<bool>,
    /// Per page: requested and not placed or dropped yet.
    pending: Vec<bool>,
    /// Per page: its need (pixels of error it takes away; 0 when unwanted), its own or lent
    /// by a wanted descendant.
    need: Vec<f32>,
    /// Per page: the last frame it had a need.
    last_needed: Vec<u64>,
    /// Per page: the pages holding a parent of its clusters.
    parents: Vec<Vec<u32>>,
    /// Per page: how many pages whose `parents` include it are resident.
    resident_children: Vec<u32>,
    /// Per page number: how many times it was released (#220). A read asked for under an
    /// earlier generation brings another mesh's page and is dropped.
    generation: Vec<u32>,
}

impl ResidentSet {
    /// `page_count` pages whose dependencies `parents` lists, `pool_pages` slots, and the
    /// pages `pinned` resident in the first slots for good.
    pub fn new(page_count: usize, pool_pages: u32, parents: Vec<Vec<u32>>, pinned: &[u32]) -> Self {
        let mut residency = Self {
            slot_of: vec![PAGE_NONE; page_count],
            page_in_slot: vec![PAGE_NONE; pool_pages as usize],
            free_slots: (pinned.len() as u32..pool_pages).rev().collect(),
            pinned: vec![false; page_count],
            pending: vec![false; page_count],
            need: vec![0.0; page_count],
            last_needed: vec![0; page_count],
            parents,
            resident_children: vec![0; page_count],
            generation: vec![0; page_count],
        };
        for (slot, &page) in pinned.iter().enumerate() {
            residency.pinned[page as usize] = true;
            residency.occupy(page, slot as u32);
        }
        residency
    }

    /// The pages holding a parent of each page's clusters, from the cluster records.
    pub fn parents_of(page_count: usize, meshlets: &[GpuMeshlet]) -> Vec<Vec<u32>> {
        let mut parents = vec![Vec::new(); page_count];
        for m in meshlets {
            // A cluster's children live in its child page: that page depends on this one.
            // Both in one page is no dependency (they load together).
            if m.child_page != PAGE_NONE && m.child_page != m.page {
                parents[m.child_page as usize].push(m.page);
            }
        }
        for list in &mut parents {
            list.sort_unstable();
            list.dedup();
        }
        parents
    }

    fn occupy(&mut self, page: u32, slot: u32) {
        self.slot_of[page as usize] = slot;
        self.page_in_slot[slot as usize] = page;
        for &parent in &self.parents[page as usize] {
            self.resident_children[parent as usize] += 1;
        }
    }

    fn vacate(&mut self, page: u32) -> u32 {
        let slot = self.slot_of[page as usize];
        self.slot_of[page as usize] = PAGE_NONE;
        self.page_in_slot[slot as usize] = PAGE_NONE;
        for &parent in &self.parents[page as usize] {
            self.resident_children[parent as usize] -= 1;
        }
        slot
    }

    fn resident(&self, page: u32) -> bool {
        self.slot_of[page as usize] != PAGE_NONE
    }

    fn parents_resident(&self, page: u32) -> bool {
        self.parents[page as usize]
            .iter()
            .all(|&p| self.resident(p))
    }

    fn evictable(&self, page: u32) -> bool {
        let p = page as usize;
        self.resident(page) && !self.pinned[p] && self.resident_children[p] == 0
    }

    /// Pages holding a page.
    pub fn resident_count(&self) -> u32 {
        self.page_in_slot
            .iter()
            .filter(|&&p| p != PAGE_NONE)
            .count() as u32
    }

    /// Takes frame `frame`'s needs (one per page) and lends each wanted page's need to its
    /// ancestors. Returns how many pages are wanted and absent.
    pub fn set_needs(&mut self, frame: u64, needs: impl Iterator<Item = f32>) -> u32 {
        for (p, need) in needs.enumerate() {
            self.need[p] = need;
            if need > 0.0 {
                self.last_needed[p] = frame;
            }
        }
        let mut lend: Vec<u32> = (0..self.slot_of.len() as u32)
            .filter(|&p| self.need[p as usize] > 0.0 && !self.resident(p))
            .collect();
        let wanted = lend.len() as u32;
        while let Some(page) = lend.pop() {
            let need = self.need[page as usize];
            for i in 0..self.parents[page as usize].len() {
                let parent = self.parents[page as usize][i] as usize;
                if self.need[parent] < need {
                    self.need[parent] = need;
                    self.last_needed[parent] = frame;
                    lend.push(parent as u32);
                }
            }
        }
        wanted
    }

    /// Places up to `budget` of the `loaded` pages, the neediest first: into free slots, or
    /// into the slot of the evictable page with the lowest need (the least recently wanted
    /// among equals) when that need is lower. A page whose parents are not all resident is
    /// dropped (requested again while wanted); pages over the budget are handed back.
    pub fn place<T>(
        &mut self,
        mut loaded: Vec<(u32, T)>,
        budget: u32,
    ) -> (Placement<T>, Vec<(u32, T)>) {
        let need = &self.need;
        loaded.sort_by(|a, b| need[b.0 as usize].total_cmp(&need[a.0 as usize]));
        let mut victims: Vec<u32> = self
            .page_in_slot
            .iter()
            .copied()
            .filter(|&p| p != PAGE_NONE && self.evictable(p))
            .collect();
        victims.sort_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            need[a]
                .total_cmp(&need[b])
                .then(self.last_needed[a].cmp(&self.last_needed[b]))
        });
        let mut victims = victims.into_iter().peekable();
        let mut placement = Placement {
            placed: Vec::new(),
            evicted: Vec::new(),
        };
        let mut later = Vec::new();
        let mut room = true;
        for (page, bytes) in loaded {
            let p = page as usize;
            if self.resident(page) || !self.parents_resident(page) || !room {
                self.pending[p] = false;
                continue;
            }
            if placement.placed.len() as u32 == budget {
                later.push((page, bytes));
                continue;
            }
            let slot = match self.free_slots.pop() {
                Some(slot) => Some(slot),
                None => loop {
                    let Some(&victim) = victims.peek() else {
                        break None;
                    };
                    if !self.evictable(victim) || self.parents[p].contains(&victim) {
                        victims.next();
                        continue;
                    }
                    if self.need[victim as usize] >= self.need[p] {
                        break None; // nothing less needed to give up
                    }
                    victims.next();
                    placement.evicted.push(victim);
                    break Some(self.vacate(victim));
                },
            };
            let Some(slot) = slot else {
                room = false;
                self.pending[p] = false;
                continue;
            };
            self.pending[p] = false;
            self.occupy(page, slot);
            placement.placed.push((page, slot, bytes));
        }
        (placement, later)
    }

    /// Up to `room` absent, wanted pages whose parents are all resident and that are not
    /// already requested, the neediest first; marks them requested.
    pub fn requests(&mut self, room: usize) -> Vec<u32> {
        let mut wanted: Vec<u32> = (0..self.slot_of.len() as u32)
            .filter(|&p| {
                self.need[p as usize] > 0.0
                    && !self.resident(p)
                    && !self.pending[p as usize]
                    && self.parents_resident(p)
            })
            .collect();
        let need = &self.need;
        wanted.sort_by(|&a, &b| need[b as usize].total_cmp(&need[a as usize]));
        wanted.truncate(room);
        for &page in &wanted {
            self.pending[page as usize] = true;
        }
        wanted
    }

    /// A requested page that will not arrive (its read failed).
    pub fn abandon(&mut self, page: u32) {
        self.pending[page as usize] = false;
    }

    /// Page `page`'s generation (see [`ResidentSet::generation`]).
    pub fn generation(&self, page: u32) -> u32 {
        self.generation[page as usize]
    }

    /// Gives pages `first..` the dependencies `parents` lists, one per page (a dynamic scene's
    /// new mesh, #220): numbers no page held, or released ones.
    pub fn add_pages(&mut self, first: u32, parents: Vec<Vec<u32>>) {
        for (k, list) in parents.into_iter().enumerate() {
            let p = first as usize + k;
            debug_assert!(self.slot_of[p] == PAGE_NONE && !self.pinned[p] && !self.pending[p]);
            self.parents[p] = list;
            self.need[p] = 0.0;
            self.last_needed[p] = 0;
        }
    }

    /// How many pages [`ResidentSet::pin`] can still make resident: the free slots, and those
    /// of the pages not pinned, which can all leave, leaves first.
    pub fn pin_room(&self) -> usize {
        self.free_slots.len()
            + self
                .page_in_slot
                .iter()
                .filter(|&&p| p != PAGE_NONE && !self.pinned[p as usize])
                .count()
    }

    /// The evictable page with the lowest need, the least recently needed among equals.
    fn least_needed_leaf(&self) -> Option<u32> {
        self.page_in_slot
            .iter()
            .copied()
            .filter(|&p| p != PAGE_NONE && self.evictable(p))
            .min_by(|&a, &b| {
                let (a, b) = (a as usize, b as usize);
                self.need[a]
                    .total_cmp(&self.need[b])
                    .then(self.last_needed[a].cmp(&self.last_needed[b]))
            })
    }

    /// Makes `page` resident for good (a dynamic scene's new root page, #220): in a free slot,
    /// or in the slot of the least needed evictable page, whatever its need. Returns the slot
    /// and the page evicted for it; `None` when every resident page is pinned or holds others'
    /// parents ([`ResidentSet::pin_room`] says how many fit).
    pub fn pin(&mut self, page: u32) -> Option<(u32, Option<u32>)> {
        let (slot, evicted) = match self.free_slots.pop() {
            Some(slot) => (slot, None),
            None => {
                let victim = self.least_needed_leaf()?;
                (self.vacate(victim), Some(victim))
            }
        };
        self.pinned[page as usize] = true;
        self.occupy(page, slot);
        Some((slot, evicted))
    }

    /// Takes `page` as wanted by `need` pixels from frame `frame` on, until the frames' own needs
    /// replace it: a dynamic scene's new page loaded for its first view (#220), which the next
    /// pages placed then evict last.
    pub fn want(&mut self, page: u32, need: f32, frame: u64) {
        self.need[page as usize] = need;
        self.last_needed[page as usize] = frame;
    }

    /// Releases pages `pages` (a dynamic scene's mesh removed, #220): every one leaves its
    /// slot, pinned or not, and is no longer wanted or requested; their numbers' generation
    /// moves on, so that a read of them still in flight is dropped when it lands. The pages
    /// must hold all their children: none outside them may stay resident.
    pub fn release_pages(&mut self, pages: std::ops::Range<u32>) {
        for page in pages.clone() {
            if self.resident(page) {
                let slot = self.vacate(page);
                self.free_slots.push(slot);
            }
        }
        for page in pages {
            let p = page as usize;
            debug_assert_eq!(self.resident_children[p], 0, "a child of page {page} stays");
            self.pinned[p] = false;
            self.pending[p] = false;
            self.need[p] = 0.0;
            self.last_needed[p] = 0;
            self.parents[p].clear();
            self.generation[p] = self.generation[p].wrapping_add(1);
        }
    }

    /// The pages a start view loads (#121), given each page's need from the view (0 for
    /// none): the absent wanted pages and every page holding a parent of their clusters, the
    /// neediest first and a parent before its children, as many as the free slots hold.
    /// Returns them and how many wanted pages did not fit.
    pub fn start_pages(&self, needs: &[f32]) -> (Vec<u32>, usize) {
        // Each wanted page lends its need to its ancestors (as `set_needs`), so a parent never
        // comes after its children; among equal needs the shallower comes first.
        let mut need = needs.to_vec();
        let mut lend: Vec<u32> = (0..need.len() as u32)
            .filter(|&p| need[p as usize] > 0.0)
            .collect();
        while let Some(page) = lend.pop() {
            let value = need[page as usize];
            for &parent in &self.parents[page as usize] {
                if need[parent as usize] < value {
                    need[parent as usize] = value;
                    lend.push(parent);
                }
            }
        }
        let depth = page_depths(&self.parents);
        let mut taken: Vec<bool> = (0..need.len() as u32).map(|p| self.resident(p)).collect();
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
        let mut pages = Vec::new();
        for &page in &wanted {
            if pages.len() == self.free_slots.len() {
                break;
            }
            if self.parents[page as usize]
                .iter()
                .all(|&p| taken[p as usize])
            {
                taken[page as usize] = true;
                pages.push(page);
            }
        }
        let missed = wanted.len() - pages.len();
        (pages, missed)
    }

    /// Makes `pages` resident in free slots, in order ([`Self::start_pages`] chose them): the
    /// slots, one per page.
    pub fn preload(&mut self, pages: &[u32]) -> Vec<u32> {
        pages
            .iter()
            .map(|&page| {
                let slot = self
                    .free_slots
                    .pop()
                    .expect("start_pages fits the free slots");
                self.occupy(page, slot);
                slot
            })
            .collect()
    }

    /// Per page, its slot or `PAGE_NONE`: the page table.
    pub fn table(&self) -> &[u32] {
        &self.slot_of
    }

    /// Panics unless the resident set is consistent and closed upwards.
    #[cfg(test)]
    fn check(&self) {
        for (page, &slot) in self.slot_of.iter().enumerate() {
            if self.pinned[page] {
                assert_ne!(slot, PAGE_NONE, "pinned page {page} left");
            }
            if slot != PAGE_NONE {
                assert_eq!(self.page_in_slot[slot as usize], page as u32);
                assert!(
                    self.parents_resident(page as u32),
                    "page {page} resident without its parents"
                );
            }
        }
        for (page, &count) in self.resident_children.iter().enumerate() {
            let actual = (0..self.slot_of.len() as u32)
                .filter(|&c| self.resident(c) && self.parents[c as usize].contains(&(page as u32)))
                .count() as u32;
            assert_eq!(count, actual, "resident children of page {page}");
        }
    }
}

/// A streamed scene's residency with its GPU buffers and its I/O thread (see the module
/// notes).
pub(crate) struct PageStreamer {
    config: StreamingConfig,
    residency: ResidentSet,
    /// Pages read and waiting for an upload budget.
    ready: Vec<(u32, Vec<u8>)>,
    need_bits: Vec<u32>,
    reads: Option<mpsc::Sender<ReadRequest>>,
    loaded: mpsc::Receiver<Loaded>,
    io_thread: Option<thread::JoinHandle<()>>,
    in_flight: u32,
    /// Per frame slot: the host-visible source of its uploads.
    staging: Vec<Buffer>,
    /// Per frame slot: the frame's needs, read back.
    pub readback: Vec<GraphBuffer>,
    /// Per frame slot: the copies its upload pass records.
    pub plans: Vec<UploadPlan>,
    /// The needs the culls write (a float's bits per page).
    pub need_buffer: GraphBuffer,
    stats: StreamingStats,
    /// The pages: each read sends the I/O thread its page's source.
    store: PageStore,
    /// The pages a start view loaded (#121), and whether a read past them was logged.
    preloaded: u32,
    read_logged: bool,
}

/// What a dynamic scene's edit brings a streamed scene's pages (#220,
/// [`PageStreamer::apply`]).
#[derive(Default)]
pub(crate) struct PageEdit {
    /// Per new mesh: its first page, and each of its pages' source and the pages holding a
    /// parent of its clusters.
    pub meshes: Vec<(u32, Vec<PageSource>, Vec<Vec<u32>>)>,
    /// The new meshes' root pages, made resident for good, each with where its bytes lie in the
    /// edit's staging.
    pub roots: Vec<(u32, u64)>,
    /// Pages the new meshes' first view wants, loaded with them as room allows: the neediest
    /// first and a parent before its children, each with its need and where its bytes lie.
    pub preload: Vec<(u32, f32, u64)>,
}

/// The copies an edit's pages need ([`PageStreamer::apply`]): (staging offset, pool offset,
/// bytes) per page loaded, and the page-table entries it changed, in page order.
#[derive(Default)]
pub(crate) struct PagePlan {
    pub pages: Vec<(u64, u64, u64)>,
    pub table: Vec<(u32, u32)>,
}

impl PageStreamer {
    /// A streamer over `store`'s pages for clusters `meshlets` (whose `page` and
    /// `child_page` index the store), with the pages `pinned_pages` already resident in the
    /// first slots.
    pub fn new(
        device: &Arc<Device>,
        config: StreamingConfig,
        store: PageStore,
        meshlets: &[GpuMeshlet],
        pinned_pages: &[u32],
    ) -> Result<Self> {
        let page_count = store.sources.len();
        let parents = ResidentSet::parents_of(page_count, meshlets);
        let buffer_bytes = (page_count.max(1) * 4) as u64;
        let mut staging = Vec::new();
        let mut readback = Vec::new();
        // Staging: the pages, then a page-table entry per eviction and per placement.
        let staging_bytes = u64::from(config.upload_pages) * (PAGE_SIZE as u64 + 8);
        for i in 0..FRAMES_IN_FLIGHT {
            staging.push(device.create_buffer(BufferDesc {
                size: staging_bytes,
                usage: vk::BufferUsageFlags::TRANSFER_SRC,
                location: MemoryLocation::CpuToGpu,
                category: MemoryCategory::Transfer,
                name: &format!("page staging {i}"),
            })?);
            let buffer = device.create_buffer(BufferDesc {
                size: buffer_bytes,
                usage: vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuToCpu,
                category: MemoryCategory::Transfer,
                name: &format!("page needs readback {i}"),
            })?;
            buffer.write(0, &vec![0_u32; page_count.max(1)]);
            readback.push(GraphBuffer::new(buffer));
        }
        let (reads, requests) = mpsc::channel::<ReadRequest>();
        let (done, loaded) = mpsc::channel::<Loaded>();
        let memory = Arc::clone(&store.memory);
        let io_thread = thread::Builder::new()
            .name("forge page reads".into())
            .spawn(move || {
                let mut open = HashMap::new();
                for (page, generation, source) in requests {
                    let mut bytes = vec![0; PAGE_SIZE];
                    let result = read_packed(&source, &memory, &mut open)
                        .and_then(|(codec, packed)| unpack(codec, &packed, &mut bytes))
                        .map(|()| bytes);
                    if done.send((page, generation, result)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            config,
            residency: ResidentSet::new(page_count, config.pool_pages, parents, pinned_pages),
            ready: Vec::new(),
            need_bits: vec![0; page_count],
            reads: Some(reads),
            loaded,
            io_thread: Some(io_thread),
            in_flight: 0,
            staging,
            readback,
            plans: (0..FRAMES_IN_FLIGHT)
                .map(|_| UploadPlan::default())
                .collect(),
            need_buffer: GraphBuffer::new(device.create_buffer(BufferDesc {
                size: buffer_bytes,
                usage: vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_SRC
                    | vk::BufferUsageFlags::TRANSFER_DST,
                location: MemoryLocation::GpuOnly,
                category: MemoryCategory::Work,
                name: "page needs",
            })?),
            stats: StreamingStats {
                pool_pages: config.pool_pages,
                ..StreamingStats::default()
            },
            store,
            preloaded: 0,
            read_logged: false,
        })
    }

    /// Loads the pages a start view's cut wants, given each page's need from it
    /// ([`ResidentSet::start_pages`]; `MeshletScene::load_start_view`, #121), into the free
    /// slots of `pool` and their entries of `table`, directly: before the first frame.
    pub fn preload(
        &mut self,
        device: &Arc<Device>,
        pool: &Buffer,
        table: &Buffer,
        needs: &[f32],
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let (pages, missed) = self.residency.start_pages(needs);
        let slots = self.residency.preload(&pages);
        let bytes = self.store.read_pages(pages.iter().copied())?;
        // One copy per run of consecutive slots: a single one before the first frame, when
        // the free slots follow the roots'.
        let mut at = 0;
        while at < slots.len() {
            let mut end = at + 1;
            while end < slots.len() && slots[end] == slots[end - 1] + 1 {
                end += 1;
            }
            device.write_buffer_staged(
                pool,
                u64::from(slots[at]) * PAGE_SIZE as u64,
                &bytes[at * PAGE_SIZE..end * PAGE_SIZE],
            )?;
            at = end;
        }
        device.write_buffer_staged(table, 0, bytemuck::cast_slice(self.residency.table()))?;
        self.preloaded = pages.len() as u32;
        tracing::info!(
            pages = pages.len(),
            mib = (pages.len() * PAGE_SIZE) >> 20,
            ms = %format_args!("{:.0}", start.elapsed().as_secs_f64() * 1e3),
            "the start view's cluster pages loaded"
        );
        if missed > 0 {
            tracing::warn!(
                missed,
                pool_pages = self.config.pool_pages,
                "the start view wants more cluster pages than the pool holds: they stream in \
                 over its first frames"
            );
        }
        Ok(())
    }

    /// Takes in a dynamic scene's new meshes' pages (#220) at frame `frame`: their sources and
    /// dependencies, their roots made resident for good and their first view's pages loaded as
    /// room allows (pages no longer wanted giving way). Returns the copies to make; nothing
    /// changes when the roots don't fit.
    pub fn apply(&mut self, edit: PageEdit, frame: u64) -> std::result::Result<PagePlan, String> {
        if self.residency.pin_room() < edit.roots.len() {
            return Err(format!(
                "{} root pages for a pool of {} pages with {} it can give up",
                edit.roots.len(),
                self.config.pool_pages,
                self.residency.pin_room()
            ));
        }
        let mut table = std::collections::BTreeMap::new();
        for (first, sources, parents) in edit.meshes {
            for (k, source) in sources.into_iter().enumerate() {
                let page = first + k as u32;
                self.store.sources[page as usize] = source;
                table.insert(page, PAGE_NONE);
            }
            self.residency.add_pages(first, parents);
        }
        let mut plan = PagePlan::default();
        let load = |plan: &mut PagePlan,
                    table: &mut std::collections::BTreeMap<u32, u32>,
                    page: u32,
                    slot: u32,
                    at: u64| {
            plan.pages
                .push((at, u64::from(slot) * PAGE_SIZE as u64, PAGE_SIZE as u64));
            table.insert(page, slot);
        };
        for &(page, at) in &edit.roots {
            let (slot, evicted) = self.residency.pin(page).expect("pin_room said it fits");
            if let Some(evicted) = evicted {
                table.insert(evicted, PAGE_NONE);
            }
            load(&mut plan, &mut table, page, slot, at);
        }
        for &(page, need, _) in &edit.preload {
            self.residency.want(page, need, frame);
        }
        let loaded: Vec<(u32, u64)> = edit.preload.iter().map(|&(p, _, at)| (p, at)).collect();
        let budget = loaded.len() as u32;
        let (placement, _) = self.residency.place(loaded, budget);
        for &evicted in &placement.evicted {
            table.insert(evicted, PAGE_NONE);
        }
        for (page, slot, at) in placement.placed {
            load(&mut plan, &mut table, page, slot, at);
        }
        plan.table = table.into_iter().collect();
        Ok(plan)
    }

    /// Releases a dynamic scene's removed mesh's pages `pages` (#220): their slots freed, their
    /// reads in flight dropped when they land, their numbers free for another mesh's. No frame
    /// may read them any more.
    pub fn release(&mut self, pages: std::ops::Range<u32>) {
        self.residency.release_pages(pages.clone());
        self.ready.retain(|(page, _)| !pages.contains(page));
        for page in pages {
            self.store.sources[page as usize] = PageSource::Absent;
        }
    }

    /// The staging buffer of frame slot `slot`.
    pub fn staging(&self, slot: usize) -> &Buffer {
        &self.staging[slot]
    }

    /// What the last frame did.
    pub fn stats(&self) -> StreamingStats {
        self.stats
    }

    /// The frame's residency work for frame slot `slot` of frame `frame_number` (see the
    /// module notes), once that slot's previous frame has completed: reads its needs,
    /// plans its uploads and evictions into its staging buffer, and sends new reads.
    pub fn begin_frame(&mut self, slot: usize, frame_number: u64) {
        let mut stats = StreamingStats {
            pool_pages: self.config.pool_pages,
            ..StreamingStats::default()
        };
        if frame_number >= FRAMES_IN_FLIGHT as u64 {
            self.readback[slot].read(0, &mut self.need_bits);
        }
        stats.wanted = self.residency.set_needs(
            frame_number,
            self.need_bits.iter().map(|&b| f32::from_bits(b)),
        );
        while let Ok((page, generation, result)) = self.loaded.try_recv() {
            self.in_flight -= 1;
            // Released since it was asked for (#220): another mesh's page now, or none.
            if generation != self.residency.generation(page) {
                continue;
            }
            match result {
                Ok(bytes) => self.ready.push((page, bytes)),
                Err(error) => {
                    tracing::warn!(page, %error, "cluster page read failed");
                    self.residency.abandon(page);
                }
            }
        }

        let (placement, later) = self
            .residency
            .place(std::mem::take(&mut self.ready), self.config.upload_pages);
        self.ready = later;
        let staging = &self.staging[slot];
        let mut plan = UploadPlan::default();
        let table_base = u64::from(self.config.upload_pages) * PAGE_SIZE as u64;
        let mut table_writes = Vec::new();
        for (i, (page, pool_slot, bytes)) in placement.placed.iter().enumerate() {
            let src = i as u64 * PAGE_SIZE as u64;
            staging.write(src, bytes);
            plan.pages.push((
                src,
                u64::from(*pool_slot) * PAGE_SIZE as u64,
                PAGE_SIZE as u64,
            ));
            table_writes.push((*page, *pool_slot));
        }
        table_writes.extend(placement.evicted.iter().map(|&page| (page, PAGE_NONE)));
        for (i, (page, value)) in table_writes.into_iter().enumerate() {
            let src = table_base + i as u64 * 4;
            staging.write(src, &[value]);
            plan.table.push((src, u64::from(page) * 4, 4));
        }
        self.plans[slot] = plan;
        stats.uploaded = placement.placed.len() as u32;
        stats.evicted = placement.evicted.len() as u32;

        let room = self.config.reads_in_flight.saturating_sub(self.in_flight) as usize;
        if let Some(reads) = &self.reads {
            for page in self.residency.requests(room) {
                let request = (
                    page,
                    self.residency.generation(page),
                    self.store.sources[page as usize].clone(),
                );
                if reads.send(request).is_err() {
                    self.residency.abandon(page);
                    continue;
                }
                self.in_flight += 1;
                stats.requested += 1;
            }
        }
        stats.reading = self.in_flight;
        stats.resident = self.residency.resident_count();
        // A fixed start view should read nothing past its pages: the first read says when
        // the view moved, or what its cut missed.
        if stats.requested > 0 && self.preloaded > 0 && !self.read_logged {
            self.read_logged = true;
            tracing::info!(
                frame = frame_number,
                requested = stats.requested,
                preloaded = self.preloaded,
                "the first cluster pages read past the start view's"
            );
        }
        self.stats = stats;
    }
}

impl Drop for PageStreamer {
    fn drop(&mut self) {
        // Closing the channel ends the I/O thread's loop after its current read.
        self.reads = None;
        if let Some(handle) = self.io_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_come_in_whole_pages() {
        let config = StreamingConfig::from_mib(256, 4);
        assert_eq!(config.pool_pages, 2048);
        assert_eq!(config.upload_pages, 32);
        assert_eq!(StreamingConfig::from_mib(0, 0).pool_pages, 1);
    }

    #[test]
    fn the_line_says_what_moved() {
        let stats = StreamingStats {
            pool_pages: 2048,
            resident: 1024,
            wanted: 30,
            requested: 12,
            reading: 20,
            uploaded: 8,
            evicted: 2,
        };
        assert_eq!(
            stats.line(),
            "streaming: 1024 of 2048 pages resident (128 of 256 MiB), 30 wanted, 12 requested, 20 reading, 8 uploaded (1.0 MiB), 2 evicted"
        );
        assert_eq!(stats.bytes_uploaded(), 8 * 128 * 1024);
    }

    /// Pages 0 (the root, pinned) → 1, 2 → 3 (needs 1 and 2) → 4; and 5 under 2.
    fn small(pool: u32) -> ResidentSet {
        ResidentSet::new(6, pool, small_parents(), &[0])
    }

    fn small_parents() -> Vec<Vec<u32>> {
        vec![vec![], vec![0], vec![0], vec![1, 2], vec![3], vec![2]]
    }

    #[test]
    fn a_start_view_loads_its_pages_with_their_parents_first() {
        assert_eq!(page_depths(&small_parents()), vec![0, 1, 1, 2, 3, 2]);
        // Page 4 wanted lends its need to 3, 1 and 2; page 5 needs less.
        let mut needs = vec![0.0; 6];
        needs[4] = 5.0;
        needs[5] = 2.0;
        assert_eq!(small(8).start_pages(&needs), (vec![1, 2, 3, 4, 5], 0));
        // Short of slots, the neediest and their parents: never a page without them.
        assert_eq!(small(4).start_pages(&needs), (vec![1, 2, 3], 2));
        assert_eq!(small(8).start_pages(&[0.0; 6]), (vec![], 0));
    }

    #[test]
    fn preloaded_pages_stay_until_given_up_like_any_other() {
        // The pool full with the root and three preloaded pages.
        let mut r = small(4);
        assert_eq!(
            r.preload(&[1, 2, 3]),
            vec![1, 2, 3],
            "the slots after the root's"
        );
        r.check();
        assert_eq!(r.resident_count(), 4);
        assert_eq!(r.table(), &[0, 1, 2, 3, PAGE_NONE, PAGE_NONE]);
        // Page 5 wanted, 3 not: 3, the only leaf, leaves for it; 1 and 2 hold its parents.
        r.set_needs(1, needs(&[(5, 1.0)]));
        let (placement, _) = r.place(vec![(5, ())], 8);
        assert_eq!(placement.evicted, vec![3]);
        assert_eq!(placement.placed.len(), 1);
        r.check();
    }

    fn needs(values: &[(usize, f32)]) -> impl Iterator<Item = f32> {
        let mut all = vec![0.0; 6];
        for &(p, v) in values {
            all[p] = v;
        }
        all.into_iter()
    }

    /// Requests, reads everything asked for and places it; returns what was placed.
    fn round(r: &mut ResidentSet, frame: u64, wanted: &[(usize, f32)]) -> Vec<u32> {
        r.set_needs(frame, needs(wanted));
        let read: Vec<(u32, ())> = r.requests(8).into_iter().map(|p| (p, ())).collect();
        let (placement, later) = r.place(read, 8);
        assert!(later.is_empty());
        r.check();
        placement.placed.iter().map(|&(p, _, ())| p).collect()
    }

    #[test]
    fn a_page_comes_after_all_its_parents_even_unwanted_ones() {
        let mut r = small(8);
        // Page 3 is wanted; page 2 is not wanted for itself but holds parents of 3.
        assert_eq!(round(&mut r, 1, &[(3, 5.0), (1, 2.0)]), vec![1, 2]);
        assert_eq!(round(&mut r, 2, &[(3, 5.0), (1, 2.0)]), vec![3]);
        assert_eq!(r.resident_count(), 4);
    }

    #[test]
    fn a_full_pool_gives_up_the_least_needed_leaf_only() {
        let mut r = small(4); // the root and three more
        round(&mut r, 1, &[(1, 3.0), (2, 3.0)]);
        round(&mut r, 2, &[(1, 3.0), (2, 3.0), (3, 2.0)]);
        assert_eq!(r.resident_count(), 4);
        // Page 5 is wanted more than 3: 3 is the only leaf that may go (1 and 2 have a
        // resident child, 0 is pinned).
        r.set_needs(3, needs(&[(1, 3.0), (2, 3.0), (3, 1.0), (5, 4.0)]));
        let (placement, _) = r.place(vec![(5, ())], 8);
        assert_eq!(placement.evicted, vec![3]);
        assert_eq!(placement.placed.len(), 1);
        r.check();
        // A page needed less than everything evictable waits.
        r.set_needs(4, needs(&[(1, 3.0), (2, 3.0), (5, 4.0), (3, 0.5)]));
        let (placement, _) = r.place(vec![(3, ())], 8);
        assert!(placement.placed.is_empty() && placement.evicted.is_empty());
        r.check();
    }

    #[test]
    fn a_new_mesh_pins_its_roots_over_the_least_needed_leaves_and_leaves_whole() {
        // The small set's pages and three numbers free for a dynamic scene's mesh (#220).
        let mut parents = small_parents();
        parents.resize(9, Vec::new());
        let mut r = ResidentSet::new(9, 4, parents, &[0]);
        assert_eq!(round(&mut r, 1, &[(1, 3.0), (2, 2.0)]), vec![1, 2]);
        // A mesh comes: its root 6 over 7 and 8.
        r.add_pages(6, vec![vec![], vec![6], vec![6]]);
        assert_eq!(
            r.pin_room(),
            3,
            "the free slot, and pages 1 and 2 may leave"
        );
        assert_eq!(r.pin(6), Some((3, None)), "the free slot");
        // The pool full: page 2, the least needed leaf, gives way to 7, wanted by the mesh's
        // first view.
        r.want(7, 4.0, 2);
        let (placement, _) = r.place(vec![(7, ())], 1);
        assert_eq!(placement.evicted, vec![2]);
        assert_eq!(placement.placed.len(), 1);
        r.check();
        // The mesh leaves whole; a read of its pages still in flight is told apart.
        let generation = r.generation(7);
        r.release_pages(6..9);
        assert_ne!(r.generation(7), generation);
        assert_eq!(r.resident_count(), 2, "the root 0 and page 1");
        r.check();
        // Another mesh takes the numbers: three roots, the third over page 1, the only page
        // that may leave.
        r.add_pages(6, vec![vec![], vec![], vec![]]);
        assert!(r.pin(6).is_some() && r.pin(7).is_some());
        assert_eq!(r.pin(8), Some((1, Some(1))));
        assert_eq!(r.pin_room(), 0);
        assert_eq!(r.pin(5), None, "every resident page pinned");
        r.check();
    }

    #[test]
    fn pages_over_the_budget_wait_for_the_next_frame() {
        let mut r = small(8);
        r.set_needs(1, needs(&[(1, 2.0), (2, 3.0)]));
        let read: Vec<(u32, ())> = r.requests(8).into_iter().map(|p| (p, ())).collect();
        let (placement, later) = r.place(read, 1);
        assert_eq!(placement.placed.len(), 1);
        assert_eq!(placement.placed[0].0, 2, "the neediest first");
        assert_eq!(later.len(), 1);
        assert!(
            r.requests(8).is_empty(),
            "a page waiting to be placed is not read again"
        );
        let (placement, later) = r.place(later, 1);
        assert_eq!(placement.placed[0].0, 1);
        assert!(later.is_empty());
        r.check();
    }
}

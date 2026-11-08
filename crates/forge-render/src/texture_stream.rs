//! Texture streaming's policy (D-105's third step): which levels each streamed texture keeps,
//! from what the frames' samples wanted (`meshlet::feedback`), within a pool of bytes.
//!
//! - **A texture keeps its levels from a top down,** whole levels (D-105's first point). Its
//!   tail, the levels of [`TAIL_TEXELS`] and less, is always resident.
//! - **Its target** is the finest level its samples wanted within the window (two seconds, as
//!   measured); once that want is older than the window, the level wanted last if it is within
//!   it; else its tail.
//! - **A plan** takes the textures down to their targets first (that frees bytes), then up,
//!   the largest gap first, each as far as the pool allows. One plan uploads at most the bytes
//!   it is given: a texture is remade whole ([`crate::material::TextureSet::remake`]), so a change
//!   costs its new levels' bytes.
//! - **Eviction by last use:** when a texture can't grow within the pool, textures wanted at
//!   least [`EVICT_MARGIN_SECONDS`] longer ago drop to their tails, the oldest first, if that
//!   makes room. The feedback looks at one pixel in 64 a frame, so two textures on screen together
//!   are rarely wanted in the same frame: without the margin they would evict each other in turn.
//!
//! The levels here count in the texture's full chain: level 0 is its finest. The feedback counts
//! from the image's current top, finer than it as negative levels, so a remade texture's wanted
//! level is the one recorded plus its top ([`LevelPlanner::wanted_in_image`]).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::Instant;

use forge_core::material::TextureId;
use forge_gpu::{Device, FRAMES_IN_FLIGHT, GpuError, Image, Result};

use crate::dds::CompressedTexture;
use crate::material::{RetiredTexture, TextureSet};
use crate::meshlet::{MeshletScene, NEED_BIAS, NO_WANT};

/// The texels on a side under which a texture's levels stay resident (D-105's second point; 64
/// to be measured).
pub const TAIL_TEXELS: u32 = 128;

/// The window over which a texture's wanted levels count, seconds (#385's measure).
pub const WINDOW_SECONDS: f64 = 2.0;

/// How much longer ago a texture must have been wanted than the one growing, to give its levels
/// up to it, seconds.
pub const EVICT_MARGIN_SECONDS: f64 = 0.5;

/// A texture the planner streams: its levels' bytes and sides.
#[derive(Clone, Debug)]
pub struct StreamedTexture {
    /// Each level's bytes, the finest first, its full chain.
    pub levels: Vec<u64>,
    /// Each level's larger side in texels, the finest first.
    pub sides: Vec<u32>,
}

impl StreamedTexture {
    /// Its first level of [`TAIL_TEXELS`] or less: the levels from it down are always resident.
    pub fn tail(&self) -> usize {
        self.sides
            .iter()
            .position(|&s| s <= TAIL_TEXELS)
            .unwrap_or(self.sides.len().saturating_sub(1))
    }

    /// The bytes of its levels from `top` down.
    pub fn bytes_from(&self, top: usize) -> u64 {
        self.levels.iter().skip(top).sum()
    }
}

/// A texture's levels to change: remade from level `to` down, where it held them from `from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelChange {
    /// The texture, as [`LevelPlanner::add`] numbered it.
    pub texture: usize,
    /// Its top before.
    pub from: usize,
    /// Its top after.
    pub to: usize,
}

struct Tracked {
    texture: StreamedTexture,
    tail: usize,
    /// Its resident top.
    top: usize,
    /// The finest level wanted, and when it was last wanted at that level or finer (seconds).
    finest: Option<(usize, f64)>,
    /// The level wanted last, and when.
    latest: Option<(usize, f64)>,
    /// It keeps the levels it has grown to ([`LevelPlanner::keep_levels`]).
    keeps: bool,
}

impl Tracked {
    /// Its target at `now`: the finest level wanted within the window, else the level wanted
    /// last if within it, else its tail; never coarser than its top if it keeps its levels.
    fn target(&self, now: f64, window: f64) -> usize {
        let within = |w: Option<(usize, f64)>| w.filter(|&(_, at)| now - at <= window);
        let target = within(self.finest)
            .or(within(self.latest))
            .map_or(self.tail, |(level, _)| level.min(self.tail));
        if self.keeps {
            target.min(self.top)
        } else {
            target
        }
    }

    fn last_use(&self) -> f64 {
        self.latest.map_or(f64::NEG_INFINITY, |(_, at)| at)
    }
}

/// Which levels each streamed texture keeps within a pool ([`Self::plan`]).
pub struct LevelPlanner {
    pool: u64,
    window: f64,
    textures: Vec<Tracked>,
    /// The bytes resident over every texture.
    held: u64,
    /// The most textures one plan changes.
    most_changes: usize,
}

impl LevelPlanner {
    /// A planner keeping the streamed textures within `pool` bytes, their wants counted over
    /// `window` seconds ([`WINDOW_SECONDS`]). The tails count against the pool too.
    pub fn new(pool: u64, window: f64) -> Self {
        Self {
            pool,
            window,
            textures: Vec::new(),
            held: 0,
            most_changes: usize::MAX,
        }
    }

    /// At most `most` textures change in one plan, besides the bytes it is given.
    pub fn set_most_changes(&mut self, most: usize) {
        self.most_changes = most;
    }

    /// Adds `texture`, resident from its tail: its number.
    pub fn add(&mut self, texture: StreamedTexture) -> usize {
        let tail = texture.tail();
        self.held += texture.bytes_from(tail);
        self.textures.push(Tracked {
            texture,
            tail,
            top: tail,
            finest: None,
            latest: None,
            keeps: false,
        });
        self.textures.len() - 1
    }

    /// Texture `texture` only ever grows: it keeps the levels it has grown to, and no other's
    /// growth takes them.
    pub fn keep_levels(&mut self, texture: usize) {
        if let Some(t) = self.textures.get_mut(texture) {
            t.keeps = true;
        }
    }

    /// Texture `texture`'s samples wanted its full chain's `level` at `now` (seconds). A want as
    /// fine as its finest, or finer, renews that one; a coarser want replaces it only once it is
    /// older than the window.
    pub fn wanted(&mut self, texture: usize, level: usize, now: f64) {
        let window = self.window;
        let Some(t) = self.textures.get_mut(texture) else {
            return;
        };
        let level = level.min(t.texture.levels.len().saturating_sub(1));
        t.latest = Some((level, now));
        let renew = match t.finest {
            Some((finest, at)) => level <= finest || now - at > window,
            None => true,
        };
        if renew {
            t.finest = Some((level, now));
        }
    }

    /// [`Self::wanted`] with `level` counted from the texture's resident top, as the feedback
    /// counts it in the image it samples: negative for levels finer than the image holds.
    pub fn wanted_in_image(&mut self, texture: usize, level: i64, now: f64) {
        if let Some(top) = self.textures.get(texture).map(|t| t.top) {
            self.wanted(texture, (top as i64 + level).max(0) as usize, now);
        }
    }

    /// Texture `texture`'s resident top.
    pub fn top(&self, texture: usize) -> usize {
        self.textures[texture].top
    }

    /// Texture `texture`'s target at `now`.
    pub fn target(&self, texture: usize, now: f64) -> usize {
        self.textures[texture].target(now, self.window)
    }

    /// The bytes resident over every texture.
    pub fn resident(&self) -> u64 {
        self.held
    }

    /// The bytes of texture `texture`'s levels from `level` down: what remaking it from there
    /// uploads.
    pub fn bytes_from(&self, texture: usize, level: usize) -> u64 {
        self.textures[texture].texture.bytes_from(level)
    }

    /// The textures it streams.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Whether it streams none.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// Texture `i` resident from `to` down: the change.
    fn set_top(&mut self, i: usize, to: usize) -> LevelChange {
        let t = &mut self.textures[i];
        self.held = self.held - t.texture.bytes_from(t.top) + t.texture.bytes_from(to);
        let change = LevelChange {
            texture: i,
            from: t.top,
            to,
        };
        t.top = to;
        change
    }

    /// Puts `changes` back, the last first: a plan whose images could not be made.
    pub fn undo(&mut self, changes: &[LevelChange]) {
        for change in changes.iter().rev() {
            self.set_top(change.texture, change.from);
        }
    }

    /// The changes at `now`, at most `upload` bytes of new levels between them (at least one
    /// change when any is due), applied to the planner's tops: the caller remakes each texture
    /// from its `to` down. Down to the targets first; then up, the largest gap first, each as far
    /// as the pool allows; a texture that can't grow at all makes room by sending textures wanted
    /// strictly longer ago back to their tails, the oldest first, when that is enough.
    pub fn plan(&mut self, now: f64, upload: u64) -> Vec<LevelChange> {
        let window = self.window;
        let n = self.textures.len();
        let mut changes: Vec<LevelChange> = Vec::new();
        let mut changed = vec![false; n];
        let mut spent = 0u64;
        // Within the bytes and the changes this plan may make: the first change always is.
        let most_changes = self.most_changes;
        let fits = |spent: u64, cost: u64, changes: &[LevelChange]| {
            changes.is_empty() || (spent + cost <= upload && changes.len() < most_changes)
        };
        // Down to the targets: textures no longer wanted as finely.
        for (i, done) in changed.iter_mut().enumerate() {
            let (to, top) = (self.textures[i].target(now, window), self.textures[i].top);
            if to > top {
                let cost = self.textures[i].texture.bytes_from(to);
                if fits(spent, cost, &changes) {
                    spent += cost;
                    changes.push(self.set_top(i, to));
                    *done = true;
                }
            }
        }
        // Up: the largest gap first, then the latest want, then the number (deterministic).
        let mut ups: Vec<(usize, usize)> = (0..n)
            .filter_map(|i| {
                let target = self.textures[i].target(now, window);
                (target < self.textures[i].top).then_some((i, target))
            })
            .collect();
        ups.sort_by(|&(a, ta), &(b, tb)| {
            let (ga, gb) = (self.textures[a].top - ta, self.textures[b].top - tb);
            gb.cmp(&ga)
                .then(
                    self.textures[b]
                        .last_use()
                        .total_cmp(&self.textures[a].last_use()),
                )
                .then(a.cmp(&b))
        });
        // The textures that may give their levels back, the least recently wanted first.
        let mut victims: Vec<usize> = (0..n)
            .filter(|&v| {
                let t = &self.textures[v];
                t.top < t.tail && !t.keeps && !changed[v]
            })
            .collect();
        victims.sort_by(|&a, &b| {
            self.textures[a]
                .last_use()
                .total_cmp(&self.textures[b].last_use())
                .then(a.cmp(&b))
        });
        let mut next_victim = 0;
        let bytes = |t: &Tracked, level: usize| t.texture.bytes_from(level);
        for (i, target) in ups {
            // Sent back to its tail by an earlier growth of this plan: not grown again in it.
            if changed[i] {
                continue;
            }
            let top = self.textures[i].top;
            // The finest level from the target up that fits the pool as it is.
            let mut to = top;
            for level in (target..top).rev() {
                let extra = bytes(&self.textures[i], level) - bytes(&self.textures[i], top);
                if self.held + extra > self.pool {
                    break;
                }
                to = level;
            }
            let mut evict = Vec::new();
            if to == top {
                let mine = self.textures[i].last_use();
                let need = bytes(&self.textures[i], target) - bytes(&self.textures[i], top);
                let mut freed = 0u64;
                let mut k = next_victim;
                while self.held - freed + need > self.pool && k < victims.len() {
                    let v = victims[k];
                    let t = &self.textures[v];
                    if v == i || changed[v] || t.top >= t.tail {
                        k += 1;
                        continue;
                    }
                    if t.last_use() + EVICT_MARGIN_SECONDS > mine {
                        break;
                    }
                    freed += bytes(t, t.top) - bytes(t, t.tail);
                    evict.push(v);
                    k += 1;
                }
                if self.held - freed + need > self.pool {
                    continue;
                }
                let cost: u64 = evict
                    .iter()
                    .map(|&v| bytes(&self.textures[v], self.textures[v].tail))
                    .sum::<u64>()
                    + bytes(&self.textures[i], target);
                if !fits(spent, cost, &changes) {
                    continue;
                }
                next_victim = k;
                for &v in &evict {
                    let tail = self.textures[v].tail;
                    spent += bytes(&self.textures[v], tail);
                    changes.push(self.set_top(v, tail));
                    changed[v] = true;
                }
                to = target;
            }
            if to == top {
                continue;
            }
            let cost = bytes(&self.textures[i], to);
            if evict.is_empty() && !fits(spent, cost, &changes) {
                continue;
            }
            spent += cost;
            changes.push(self.set_top(i, to));
            changed[i] = true;
        }
        changes
    }
}

/// A texture's first level of [`TAIL_TEXELS`] or less: what a scene uploads of it at the start
/// when it streams it ([`TextureStream`]).
pub fn tail_level(texture: &CompressedTexture) -> usize {
    streamed(texture).tail()
}

fn streamed(texture: &CompressedTexture) -> StreamedTexture {
    let side = texture.width.max(texture.height);
    StreamedTexture {
        levels: texture.levels.iter().map(|l| l.len() as u64).collect(),
        sides: (0..texture.levels.len() as u32)
            .map(|k| (side >> k).max(1))
            .collect(),
    }
}

/// `texture`'s levels from `top` down, as a texture of their own.
fn from_level(texture: &CompressedTexture, top: usize) -> CompressedTexture {
    let top = top.min(texture.levels.len().saturating_sub(1));
    CompressedTexture {
        name: texture.name.clone(),
        width: (texture.width >> top).max(1),
        height: (texture.height >> top).max(1),
        format: texture.format,
        srgb: texture.srgb,
        levels: texture.levels[top..].to_vec(),
    }
}

/// The most textures a [`TextureStream`] changes in one frame: each moves its rows and takes a
/// slot, whatever its bytes.
const MOST_CHANGES: usize = 96;

/// After a plan whose images could not be made (out of memory, most likely), the seconds before
/// the next.
const RETRY_SECONDS: f64 = 2.0;

/// A texture named by more material rows than this only ever grows: moving it to a new slot
/// writes every one of its rows, once per frame slot.
const MANY_ROWS: usize = 64;

/// The rows a frame moves at most (one texture at least): the rest of a plan's images wait for
/// the next frames.
const MOST_ROWS_A_FRAME: usize = 16_384;

/// One texture a [`TextureStream`] streams.
struct StreamEntry {
    id: TextureId,
    /// Every level, in the CPU's memory until D-105's files.
    source: Arc<CompressedTexture>,
    /// Its bindless slot now.
    slot: u32,
    /// The top level the image in that slot holds: the feedback counts from it.
    top: usize,
}

/// One texture's new image, waiting for its frame to take its place.
struct Landing {
    change: LevelChange,
    image: Image,
    bytes: u64,
    texture: CompressedTexture,
}

/// A plan's textures, each with the level its new image starts from.
type UploadJob = Vec<(Arc<CompressedTexture>, usize)>;
/// A plan's images with their bytes, and its textures' descriptions (their levels left out).
type UploadDone = Result<(Vec<(Image, u64)>, Vec<CompressedTexture>)>;

/// The thread that makes a plan's images: the frame neither copies their texels nor waits for
/// their transfer.
struct Uploader {
    jobs: Option<Sender<UploadJob>>,
    done: Receiver<UploadDone>,
    thread: Option<JoinHandle<()>>,
}

impl Uploader {
    fn new(device: Arc<Device>) -> Result<Self> {
        let (jobs, todo) = channel::<UploadJob>();
        let (made, done) = channel::<UploadDone>();
        let thread = std::thread::Builder::new()
            .name("texture stream".to_owned())
            .spawn(move || {
                while let Ok(job) = todo.recv() {
                    let mut data: Vec<CompressedTexture> = job
                        .iter()
                        .map(|(source, top)| from_level(source, *top))
                        .collect();
                    let uploaded = crate::material::upload_compressed(&device, &data);
                    for texture in &mut data {
                        texture.levels = Vec::new();
                    }
                    if made.send(uploaded.map(|images| (images, data))).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| GpuError::Unsupported(format!("the texture stream's worker: {e}")))?;
        Ok(Self {
            jobs: Some(jobs),
            done,
            thread: Some(thread),
        })
    }
}

impl Drop for Uploader {
    fn drop(&mut self) {
        // Closing the jobs ends its loop.
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// D-105 joined up: each frame the textures' feedback (`meshlet::feedback`) goes to the planner,
/// what it plans is made on a worker ([`Uploader`]), the images take their textures' places
/// ([`TextureSet::replace_compressed`]), the rows move to the new slots
/// ([`MeshletScene::remap_texture`]) and the old images retire once no frame can read them.
///
/// **Only what reaches its slot through the scene's material rows can stream.** Decals, holograms,
/// particles and the sky copy a texture's slot once and never follow it, and the feedback doesn't
/// see what they sample: those stay out of the stream, resident and whole.
pub struct TextureStream {
    planner: LevelPlanner,
    textures: Vec<StreamEntry>,
    /// The bytes one plan may remake.
    upload: u64,
    start: Option<Instant>,
    uploader: Option<Uploader>,
    /// The plan the worker is on: one at a time.
    uploading: Option<Vec<LevelChange>>,
    /// A plan's images made and not yet in place ([`MOST_ROWS_A_FRAME`]).
    landing: VecDeque<Landing>,
    /// The material rows moved so far.
    rows_moved: u64,
    /// When the next plan may be made (seconds), after one that failed.
    retry_at: f64,
    /// The plans that failed.
    failed: u64,
    /// Old images waiting for every frame slot's table to take the new slot, by the remap's
    /// generation.
    waiting: VecDeque<(u64, RetiredTexture)>,
    /// Old images no table names any more, with the frame from which none in flight can use them.
    dropping: VecDeque<(u64, RetiredTexture)>,
    /// Old images whose rows did not move (a slideshow shows them): kept for good.
    kept: Vec<RetiredTexture>,
    remade: u64,
    uploaded: u64,
    /// Seconds spent so far planning, remaking and moving rows, and the most changes in one plan.
    plan_s: f64,
    remake_s: f64,
    remap_s: f64,
    changes_max: usize,
}

impl TextureStream {
    /// A stream within `pool` bytes, remaking at most `upload` bytes a frame.
    pub fn new(pool: u64, upload: u64) -> Self {
        let mut planner = LevelPlanner::new(pool, WINDOW_SECONDS);
        planner.set_most_changes(MOST_CHANGES);
        Self {
            planner,
            textures: Vec::new(),
            upload,
            start: None,
            uploader: None,
            uploading: None,
            landing: VecDeque::new(),
            rows_moved: 0,
            retry_at: 0.0,
            failed: 0,
            waiting: VecDeque::new(),
            dropping: VecDeque::new(),
            kept: Vec::new(),
            remade: 0,
            uploaded: 0,
            plan_s: 0.0,
            remake_s: 0.0,
            remap_s: 0.0,
            changes_max: 0,
        }
    }

    /// [`Self::new`] with `pool_mib` and 32 MiB a frame; `FORGE_TEXTURE_POOL_MIB` and
    /// `FORGE_TEXTURE_UPLOAD_MIB` in their place.
    pub fn from_env(pool_mib: u64) -> Self {
        let mib = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default)
                << 20
        };
        Self::new(
            mib("FORGE_TEXTURE_POOL_MIB", pool_mib),
            mib("FORGE_TEXTURE_UPLOAD_MIB", 32),
        )
    }

    /// Streams texture `id`, whose image holds `source`'s levels from its tail
    /// ([`tail_level`]) down.
    pub fn add(&mut self, id: TextureId, source: Arc<CompressedTexture>) {
        let number = self.planner.add(streamed(&source));
        self.textures.push(StreamEntry {
            id,
            source,
            slot: 0,
            top: self.planner.top(number),
        });
    }

    /// The textures it streams.
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Whether it streams none.
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// The bytes the streamed textures hold now, the images remade and the bytes they uploaded.
    pub fn stats(&self) -> (u64, u64, u64) {
        (self.planner.resident(), self.remade, self.uploaded)
    }

    /// Learns each texture's slot.
    pub(crate) fn bind(&mut self, set: &TextureSet) {
        for entry in &mut self.textures {
            entry.slot = set.sampled(entry.id);
        }
    }

    /// Marks the textures that many material rows name ([`MANY_ROWS`]): `rows_of` gives a slot's
    /// rows.
    pub(crate) fn keep_widely_used(&mut self, rows_of: impl Fn(u32) -> usize) {
        let (mut kept, mut rows, mut most) = (0usize, 0usize, 0usize);
        let mut rows_kept = 0usize;
        for (t, entry) in self.textures.iter().enumerate() {
            let named = rows_of(entry.slot);
            rows += named;
            most = most.max(named);
            if named > MANY_ROWS {
                self.planner.keep_levels(t);
                kept += 1;
                rows_kept += named;
            }
        }
        tracing::info!(
            textures = self.textures.len(),
            kept,
            rows,
            rows_kept,
            most,
            "the streamed textures that many material rows name only grow (D-105)"
        );
    }

    /// One frame's work: `wants`, the feedback read this frame, per bindless index, if any.
    pub(crate) fn step(
        &mut self,
        scene: &mut MeshletScene,
        wants: Option<&[u32]>,
        frame: u64,
    ) -> Result<()> {
        let now = self
            .start
            .get_or_insert_with(Instant::now)
            .elapsed()
            .as_secs_f64();
        if let Some(wants) = wants {
            for (t, entry) in self.textures.iter().enumerate() {
                let level = wants.get(entry.slot as usize).copied().unwrap_or(NO_WANT);
                if level != NO_WANT {
                    // From the image the feedback sampled: a plan on its way up hasn't replaced
                    // it yet, whatever the planner's top says.
                    let wanted = entry.top as i64 + i64::from(level) - i64::from(NEED_BIAS);
                    self.planner.wanted(t, wanted.max(0) as usize, now);
                }
            }
        }
        self.land(scene, now)?;
        if self.uploading.is_none()
            && self.landing.is_empty()
            && wants.is_some()
            && now >= self.retry_at
        {
            let begun = Instant::now();
            let changes = self.planner.plan(now, self.upload);
            self.plan_s += begun.elapsed().as_secs_f64();
            self.changes_max = self.changes_max.max(changes.len());
            if let (false, Some(set)) = (changes.is_empty(), scene.textures_mut()) {
                if self.uploader.is_none() {
                    self.uploader = Some(Uploader::new(set.device().clone())?);
                }
                let job: UploadJob = changes
                    .iter()
                    .map(|c| (self.textures[c.texture].source.clone(), c.to))
                    .collect();
                if let Some(jobs) = self.uploader.as_ref().and_then(|u| u.jobs.as_ref()) {
                    jobs.send(job).map_err(|_| worker_gone())?;
                }
                self.uploading = Some(changes);
            }
        }
        while let Some(&(generation, _)) = self.waiting.front() {
            if !scene.texture_remap_done(generation) {
                break;
            }
            if let Some((_, retired)) = self.waiting.pop_front() {
                self.dropping
                    .push_back((frame + FRAMES_IN_FLIGHT as u64 + 1, retired));
            }
        }
        while self.dropping.front().is_some_and(|&(at, _)| at <= frame) {
            self.dropping.pop_front();
        }
        if frame.is_multiple_of(600) {
            tracing::info!(
                frame,
                textures = self.textures.len(),
                held_mib = self.planner.resident() >> 20,
                remade = self.remade,
                uploaded_mib = self.uploaded >> 20,
                waiting = self.waiting.len() + self.dropping.len(),
                kept = self.kept.len(),
                failed = self.failed,
                rows_moved = self.rows_moved,
                plan_s = self.plan_s,
                remake_s = self.remake_s,
                remap_s = self.remap_s,
                changes_max = self.changes_max,
                "texture stream (D-105)"
            );
        }
        Ok(())
    }

    /// The plan on its way up: once the worker has made its images, they take their textures'
    /// places and the rows move to their slots, as many as [`MOST_ROWS_A_FRAME`] a frame. A plan
    /// that failed is put back and the next one waits; the textures stay as they are.
    fn land(&mut self, scene: &mut MeshletScene, now: f64) -> Result<()> {
        if let (Some(uploader), true) = (&self.uploader, self.uploading.is_some()) {
            let made = match uploader.done.try_recv() {
                Ok(made) => Some(made),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return Err(worker_gone()),
            };
            if let Some(made) = made {
                let changes = self.uploading.take().unwrap_or_default();
                match made {
                    Ok((images, data)) => {
                        for ((change, (image, bytes)), texture) in
                            changes.into_iter().zip(images).zip(data)
                        {
                            self.landing.push_back(Landing {
                                change,
                                image,
                                bytes,
                                texture,
                            });
                        }
                    }
                    Err(error) => {
                        self.planner.undo(&changes);
                        self.retry_at = now + RETRY_SECONDS;
                        self.failed += 1;
                        tracing::warn!(%error, textures = changes.len(), "a texture stream's plan could not be made");
                    }
                }
            }
        }
        let mut rows = 0;
        while rows < MOST_ROWS_A_FRAME {
            let Some(landing) = self.landing.pop_front() else {
                break;
            };
            let Some(set) = scene.textures_mut() else {
                break;
            };
            let begun = Instant::now();
            let entry = &mut self.textures[landing.change.texture];
            let retired =
                set.replace_compressed(entry.id, landing.image, landing.bytes, &landing.texture)?;
            entry.slot = set.sampled(entry.id);
            entry.top = landing.change.to;
            let new = entry.slot;
            self.remade += 1;
            self.uploaded += landing.bytes;
            self.remake_s += begun.elapsed().as_secs_f64();
            let begun = Instant::now();
            // At least one texture's, so a texture with more rows than the budget still lands.
            let named = scene.texture_rows(retired.slot());
            rows += named.max(1);
            self.rows_moved += named as u64;
            match scene.remap_texture(retired.slot(), new) {
                Some(generation) => self.waiting.push_back((generation, retired)),
                None => self.kept.push(retired),
            }
            self.remap_s += begun.elapsed().as_secs_f64();
        }
        Ok(())
    }
}

/// The error of a [`TextureStream`] whose worker has stopped.
fn worker_gone() -> GpuError {
    GpuError::Unsupported("the texture stream's worker stopped".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square RGBA8 texture of `side` texels at its finest, every level down to 1.
    fn square(side: u32) -> StreamedTexture {
        let sides: Vec<u32> = (0..=side.ilog2()).map(|k| side >> k).collect();
        StreamedTexture {
            levels: sides
                .iter()
                .map(|&s| u64::from(s) * u64::from(s) * 4)
                .collect(),
            sides,
        }
    }

    #[test]
    fn a_texture_starts_at_its_tail_and_grows_to_what_was_wanted() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = p.add(square(1024));
        // 1024, 512, 256, 128: the tail starts at level 3.
        assert_eq!(p.top(t), 3);
        assert_eq!(p.resident(), square(1024).bytes_from(3));
        assert!(p.plan(0.0, u64::MAX).is_empty());
        p.wanted(t, 1, 0.0);
        assert_eq!(
            p.plan(0.1, u64::MAX),
            vec![LevelChange {
                texture: t,
                from: 3,
                to: 1
            }]
        );
        // A want coarser than the tail changes nothing.
        let u = p.add(square(1024));
        p.wanted(u, 6, 0.2);
        assert!(p.plan(0.3, u64::MAX).is_empty());
    }

    #[test]
    fn a_texture_that_keeps_its_levels_never_shrinks_and_is_never_evicted() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = p.add(square(512));
        p.keep_levels(t);
        p.wanted(t, 0, 0.0);
        p.plan(0.0, u64::MAX);
        assert_eq!(p.top(t), 0);
        // Long unwanted: an ordinary texture would be back at its tail.
        assert!(p.plan(100.0, u64::MAX).is_empty());
        assert_eq!(p.top(t), 0);
        // Another's growth can't take its levels.
        let mut q = LevelPlanner::new(
            square(512).bytes_from(0) + square(512).bytes_from(1),
            WINDOW_SECONDS,
        );
        let (a, b) = (q.add(square(512)), q.add(square(512)));
        q.keep_levels(a);
        q.wanted(a, 0, 0.0);
        q.plan(0.0, u64::MAX);
        q.wanted(b, 0, 50.0);
        q.plan(50.0, u64::MAX);
        assert_eq!(q.top(a), 0);
    }

    #[test]
    fn a_plan_put_back_leaves_the_tops_and_the_bytes_as_they_were() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = p.add(square(1024));
        let (top, held) = (p.top(t), p.resident());
        p.wanted(t, 0, 0.0);
        let changes = p.plan(0.0, u64::MAX);
        assert_ne!(p.top(t), top);
        p.undo(&changes);
        assert_eq!((p.top(t), p.resident()), (top, held));
    }

    #[test]
    fn a_texture_not_wanted_for_a_window_goes_back_to_its_tail() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = p.add(square(512));
        p.wanted(t, 0, 0.0);
        p.plan(0.0, u64::MAX);
        assert_eq!(p.top(t), 0);
        // Wanted coarser since: its finest want holds for the window after it was made.
        p.wanted(t, 1, 1.5);
        assert!(p.plan(1.9, u64::MAX).is_empty());
        // Once that is older than the window, the level wanted last.
        assert_eq!(
            p.plan(3.0, u64::MAX),
            vec![LevelChange {
                texture: t,
                from: 0,
                to: 1
            }]
        );
        // Then nothing for longer than the window: back to its tail (level 2, 128 texels).
        assert_eq!(
            p.plan(3.6, u64::MAX),
            vec![LevelChange {
                texture: t,
                from: 1,
                to: 2
            }]
        );
    }

    #[test]
    fn the_wanted_level_counts_from_the_images_top() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = p.add(square(1024));
        // The image holds levels 3 and down: its own level 0 is the chain's level 3.
        p.wanted_in_image(t, 0, 0.0);
        assert!(p.plan(0.0, u64::MAX).is_empty());
        // Two levels finer than the image holds: the chain's level 1.
        p.wanted_in_image(t, -2, 0.1);
        p.plan(0.1, u64::MAX);
        assert_eq!(p.top(t), 1);
        // Finer than the chain has: its finest.
        p.wanted_in_image(t, -5, 0.2);
        p.plan(0.2, u64::MAX);
        assert_eq!(p.top(t), 0);
        p.wanted_in_image(t, 2, 0.3);
        // Still within the window of the want of level 0.
        assert!(p.plan(0.3, u64::MAX).is_empty());
    }

    #[test]
    fn the_pool_holds_growth_back_and_evicts_the_least_recently_wanted() {
        let full = square(1024).bytes_from(0);
        let tail = square(1024).bytes_from(3);
        // Room for one texture whole and the other's tail.
        let mut p = LevelPlanner::new(full + tail, WINDOW_SECONDS);
        let (a, b) = (p.add(square(1024)), p.add(square(1024)));
        p.wanted(a, 0, 0.0);
        p.plan(0.0, u64::MAX);
        assert_eq!((p.top(a), p.top(b)), (0, 3));
        assert!(p.resident() <= full + tail);
        // B is wanted later: A, wanted less recently, gives its levels back.
        p.wanted(b, 0, 1.0);
        let changes = p.plan(1.0, u64::MAX);
        assert_eq!((p.top(a), p.top(b)), (3, 0), "{changes:?}");
        assert!(p.resident() <= full + tail);
        // Both wanted in the same frame: neither evicts the other; the one resident keeps its levels.
        p.wanted(a, 0, 1.5);
        p.wanted(b, 0, 1.5);
        assert!(p.plan(1.5, u64::MAX).is_empty());
        assert_eq!((p.top(a), p.top(b)), (3, 0));
        assert!(p.resident() <= full + tail);
    }

    #[test]
    fn a_plan_uploads_at_most_its_bytes() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let ids: Vec<usize> = (0..4).map(|_| p.add(square(1024))).collect();
        for &t in &ids {
            p.wanted(t, 0, 0.0);
        }
        let one = square(1024).bytes_from(0);
        // Room for two remakes a plan: two now, two next.
        assert_eq!(p.plan(0.0, 2 * one).len(), 2);
        assert_eq!(p.plan(0.0, 2 * one).len(), 2);
        assert!(p.plan(0.0, 2 * one).is_empty());
        // Less than one remake's bytes still makes one: a plan is never stuck.
        let mut q = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = q.add(square(1024));
        q.wanted(t, 0, 0.0);
        assert_eq!(q.plan(0.0, 1).len(), 1);
    }
}

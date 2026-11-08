//! Texture streaming's policy (D-105's third step): which levels each streamed texture keeps,
//! from what the frames' samples wanted (`meshlet::feedback`), within a pool of bytes.
//!
//! - **A texture keeps its levels from a top down,** whole levels (D-105's first point). Its
//!   tail, the levels of [`TAIL_TEXELS`] and less, is always resident.
//! - **Its want** is the finest level its samples wanted within the window (two seconds, as
//!   measured); once that want is older than the window, the level wanted last if it is within
//!   it. A texture grows to its want; nothing shrinks it on its own.
//! - **A texture keeps its levels until the room is needed.** Shrinking is not free: a smaller
//!   image is made and uploaded and the rows moved, and going back pays it again. A plan grows
//!   textures within the pool, the largest gap first, one plan's bytes at most.
//! - **Room comes from the coldest, a level at a time,** among textures wanted at least
//!   [`EVICT_MARGIN_SECONDS`] longer ago: each loses its top level before any loses a second, as
//!   a texture's top level is three quarters of its bytes. The feedback looks at one pixel in 64
//!   a frame, so two textures on screen together are rarely wanted in the same frame: without
//!   the margin they would evict each other in turn.
//! - **The pool follows the card:** [`TextureStream`] sets it from the memory the system gives
//!   the process, so spare memory holds textures and other work gets it back.
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
    /// It never gives its levels back ([`LevelPlanner::keep_levels`]).
    keeps: bool,
}

impl Tracked {
    /// What it should hold at `now`: the finest level wanted within the window, else the level
    /// wanted last if within it; never coarser than what it holds.
    fn target(&self, now: f64, window: f64) -> usize {
        let within = |w: Option<(usize, f64)>| w.filter(|&(_, at)| now - at <= window);
        within(self.finest)
            .or(within(self.latest))
            .map_or(self.top, |(level, _)| level.min(self.tail))
            .min(self.top)
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

    /// Texture `texture` never gives its levels back: moving it to a smaller image would be
    /// dearer than the room it frees.
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

    /// The pool, bytes: the next plan gives levels back if more than this is resident.
    pub fn set_pool(&mut self, pool: u64) {
        self.pool = pool;
    }

    /// The bytes of every texture's tail: the least the pool can hold.
    pub fn tails(&self) -> u64 {
        self.textures
            .iter()
            .map(|t| t.texture.bytes_from(t.tail))
            .sum()
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

    /// The textures that can give levels back, the coldest first; those that
    /// [keep them](Self::keep_levels) only if `last_resort`, after all the others.
    fn coldest_first(&self, last_resort: bool) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.textures.len())
            .filter(|&v| {
                let t = &self.textures[v];
                t.top < t.tail && (last_resort || !t.keeps)
            })
            .collect();
        order.sort_by(|&a, &b| {
            let (ta, tb) = (&self.textures[a], &self.textures[b]);
            ta.keeps
                .cmp(&tb.keeps)
                .then(ta.last_use().total_cmp(&tb.last_use()))
                .then(a.cmp(&b))
        });
        order
    }

    /// Who gives `need` bytes back, from `order` (the coldest first) among textures wanted at
    /// least [`EVICT_MARGIN_SECONDS`] before `mine` and not in `changed`: a level each, then a
    /// second each, and so on. The bytes found (less than `need` if there aren't enough) and
    /// each giver with the top it is left with.
    fn room(
        &self,
        order: &[usize],
        need: u64,
        mine: f64,
        changed: &[bool],
    ) -> (u64, Vec<(usize, usize)>) {
        let mut freed = 0u64;
        let mut givers: Vec<(usize, usize)> = Vec::new();
        let mut at: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        // Each kind is sorted by itself: past its first warm texture, only the other kind is left.
        let kept = order.partition_point(|&v| !self.textures[v].keeps);
        let (others, keepers) = order.split_at(kept);
        for pass in 0.. {
            let mut any = false;
            for part in [others, keepers] {
                for &v in part {
                    let t = &self.textures[v];
                    if t.last_use() + EVICT_MARGIN_SECONDS > mine {
                        break;
                    }
                    let level = t.top + pass;
                    if changed[v] || level >= t.tail {
                        continue;
                    }
                    any = true;
                    freed += t.texture.bytes_from(level) - t.texture.bytes_from(level + 1);
                    match at.get(&v) {
                        Some(&k) => givers[k].1 = level + 1,
                        None => {
                            at.insert(v, givers.len());
                            givers.push((v, level + 1));
                        }
                    }
                    if freed >= need {
                        return (freed, givers);
                    }
                }
            }
            if !any {
                break;
            }
        }
        (freed, givers)
    }

    /// The changes at `now`, at most `upload` bytes of new levels between them (at least one
    /// change when any is due), applied to the planner's tops: the caller remakes each texture
    /// from its `to` down. Above the pool (it shrank), the coldest give levels back; then each
    /// texture grows to what it wants, the largest gap first, within the pool as far as it can,
    /// and with room from the coldest ([`Self::room`]) for the rest.
    pub fn plan(&mut self, now: f64, upload: u64) -> Vec<LevelChange> {
        let window = self.window;
        let n = self.textures.len();
        let mut changes: Vec<LevelChange> = Vec::new();
        let mut changed = vec![false; n];
        let mut spent = 0u64;
        // Within the changes this plan may make, and its bytes: the first change may go over
        // those, or no texture larger than a plan's bytes would ever change.
        let most_changes = self.most_changes;
        let fits = |spent: u64, cost: u64, count: usize, changes: &[LevelChange]| {
            changes.len() + count <= most_changes && (changes.is_empty() || spent + cost <= upload)
        };
        let bytes = |t: &Tracked, level: usize| t.texture.bytes_from(level);
        // Sorted the first time room is wanted: most plans don't need it.
        let mut order: Option<Vec<usize>> = None;
        if self.held > self.pool {
            // The memory is wanted elsewhere: everything can be asked.
            let list = self.coldest_first(true);
            let (_, givers) = self.room(&list, self.held - self.pool, now, &changed);
            for (v, to) in givers {
                let cost = bytes(&self.textures[v], to);
                if !fits(spent, cost, 1, &changes) {
                    break;
                }
                spent += cost;
                changes.push(self.set_top(v, to));
                changed[v] = true;
            }
        }
        // The largest gap first, then the latest want, then the number (deterministic).
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
        for (i, target) in ups {
            if changes.len() >= most_changes || (!changes.is_empty() && spent >= upload) {
                break;
            }
            // It gave levels up earlier in this plan: not grown again in it.
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
            let mut givers = Vec::new();
            // Finer with room from the coldest: its want, or failing that up to two levels less.
            if to != target {
                let mine = self.textures[i].last_use();
                let list = order.get_or_insert_with(|| self.coldest_first(false));
                for level in (target..to).take(3) {
                    let need = bytes(&self.textures[i], level) - bytes(&self.textures[i], top);
                    let short = (self.held + need).saturating_sub(self.pool);
                    let (freed, found) = self.room(list, short, mine, &changed);
                    if freed < short {
                        continue;
                    }
                    let cost = found
                        .iter()
                        .map(|&(v, t)| bytes(&self.textures[v], t))
                        .sum::<u64>()
                        + bytes(&self.textures[i], level);
                    if fits(spent, cost, found.len() + 1, &changes) {
                        givers = found;
                        to = level;
                        break;
                    }
                }
            }
            if to == top {
                continue;
            }
            if givers.is_empty() && !fits(spent, bytes(&self.textures[i], to), 1, &changes) {
                continue;
            }
            for (v, t) in givers {
                spent += bytes(&self.textures[v], t);
                changes.push(self.set_top(v, t));
                changed[v] = true;
            }
            spent += bytes(&self.textures[i], to);
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
const MOST_CHANGES: usize = 256;

/// After a plan whose images could not be made (out of memory, most likely), the seconds before
/// the next.
const RETRY_SECONDS: f64 = 2.0;

/// A texture named by more material rows than this never gives its levels back: moving it to a
/// new slot writes every one of its rows, once per frame slot.
const MANY_ROWS: usize = 64;

/// The rows a frame moves at most (one texture at least): the rest of a plan's images wait for
/// the next frames.
const MOST_ROWS_A_FRAME: usize = 16_384;

/// The share of the memory the system gives the process that the pool may bring it up to.
const BUDGET_SHARE: f64 = 0.92;

/// Frames between two looks at the memory the system gives the process.
const BUDGET_FRAMES: u64 = 60;

/// How far the pool must be over what the memory allows before it is lowered, bytes: giving
/// levels back takes frames to show.
const BUDGET_SLACK: i64 = 128 << 20;

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
    /// The pool at its largest, bytes.
    most_pool: u64,
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
            most_pool: pool,
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

    /// The textures that keep their levels ([`Self::keep_widely_used`]) go up whole before the
    /// first frame: growing one later writes every row that names it, up to 1.6 million, in the
    /// frames the world is first seen; here nothing is in flight to wait for.
    pub(crate) fn raise_widely_used(&mut self, scene: &mut MeshletScene) -> Result<()> {
        let raised: Vec<usize> = (0..self.textures.len())
            .filter(|&t| self.planner.textures[t].keeps && self.planner.top(t) > 0)
            .collect();
        let Some(set) = scene.textures_mut() else {
            return Ok(());
        };
        let data: Vec<CompressedTexture> = raised
            .iter()
            .map(|&t| from_level(&self.textures[t].source, 0))
            .collect();
        let device = set.device().clone();
        let images = crate::material::upload_compressed(&device, &data)?;
        let before = self.planner.resident();
        let mut moved = Vec::with_capacity(raised.len());
        for ((&t, (image, bytes)), texture) in raised.iter().zip(images).zip(&data) {
            let entry = &mut self.textures[t];
            let retired = set.replace_compressed(entry.id, image, bytes, texture)?;
            entry.slot = set.sampled(entry.id);
            entry.top = 0;
            self.planner.set_top(t, 0);
            moved.push((retired, entry.slot));
        }
        for (retired, new) in moved {
            scene.remap_texture(retired.slot(), new);
            drop(retired);
        }
        scene.write_streamed_rows();
        tracing::info!(
            textures = raised.len(),
            mib = (self.planner.resident() - before) >> 20,
            "the widely used textures whole from the start (D-105)"
        );
        Ok(())
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
        if frame.is_multiple_of(BUDGET_FRAMES)
            && let Some(set) = scene.textures_mut()
        {
            let report = set.device().memory_report();
            if let Some((used, budget)) = report.device_local() {
                // Spare memory holds textures; what the rest of the frame takes comes back. The
                // free space inside the allocator's blocks counts as free: giving levels back
                // makes holes there long before it empties a block.
                let holes = report.reserved.saturating_sub(report.total_allocated());
                let room =
                    (budget as f64 * BUDGET_SHARE) as i64 - used.saturating_sub(holes) as i64;
                let held = self.planner.resident() as i64;
                let least = self.planner.tails() as i64 + (64 << 20);
                let pool = (held + room).clamp(least, self.most_pool.max(least as u64) as i64);
                let now_pool = self.planner.pool as i64;
                if pool > now_pool || pool < now_pool - BUDGET_SLACK {
                    self.planner.set_pool(pool as u64);
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
                pool_mib = self.planner.pool >> 20,
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
    fn a_texture_keeps_its_levels_until_the_room_is_needed() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let t = p.add(square(512));
        p.wanted(t, 0, 0.0);
        p.plan(0.0, u64::MAX);
        assert_eq!(p.top(t), 0);
        // Long unwanted, with room to spare: nothing changes.
        assert!(p.plan(100.0, u64::MAX).is_empty());
        assert_eq!(p.top(t), 0);
    }

    #[test]
    fn a_pool_made_smaller_takes_the_coldest_top_levels_first() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let (a, b) = (p.add(square(1024)), p.add(square(1024)));
        p.wanted(a, 0, 0.0);
        p.plan(0.0, u64::MAX);
        p.wanted(b, 0, 5.0);
        p.plan(5.0, u64::MAX);
        assert_eq!((p.top(a), p.top(b)), (0, 0));
        // A's top level alone is enough: B, wanted later, keeps all of its own.
        p.set_pool(p.resident() - square(1024).levels[0]);
        p.plan(9.0, u64::MAX);
        assert_eq!((p.top(a), p.top(b)), (1, 0));
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
    fn a_texture_grows_as_far_as_the_cold_ones_can_make_room() {
        // One 1024 whole in the pool, and a colder one holding it: B can't have its want (a
        // whole 2048) but a coarser level fits.
        let full = square(1024).bytes_from(0);
        let mut p = LevelPlanner::new(full + square(2048).bytes_from(4), WINDOW_SECONDS);
        let (a, b) = (p.add(square(1024)), p.add(square(2048)));
        p.wanted(a, 0, 0.0);
        p.plan(0.0, u64::MAX);
        assert_eq!(p.top(a), 0);
        p.wanted(b, 0, 5.0);
        p.plan(5.0, u64::MAX);
        assert!(p.top(b) < 4, "{}", p.top(b));
        assert!(p.resident() <= full + square(2048).bytes_from(4));
    }

    #[test]
    fn a_pool_short_of_memory_takes_what_keeps_its_levels_last() {
        let mut p = LevelPlanner::new(u64::MAX, WINDOW_SECONDS);
        let (a, b) = (p.add(square(1024)), p.add(square(1024)));
        p.keep_levels(a);
        p.wanted(a, 0, 0.0);
        p.wanted(b, 0, 0.0);
        p.plan(0.0, u64::MAX);
        // More than B can give: A's levels go too, B's first.
        p.set_pool(square(1024).bytes_from(3) * 2);
        p.plan(9.0, u64::MAX);
        assert_eq!((p.top(a), p.top(b)), (3, 3));
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

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
//! - **Eviction by last use:** when a texture can't grow within the pool, textures wanted
//!   strictly longer ago drop to their tails, the oldest first, if that makes room; textures
//!   wanted in the same frame never evict each other (no thrashing).
//!
//! The levels here count in the texture's full chain: level 0 is its finest. The feedback counts
//! from the image's current top, so a remade texture's wanted level is the one recorded plus its
//! top ([`LevelPlanner::wanted_in_image`]).

/// The texels on a side under which a texture's levels stay resident (D-105's second point; 64
/// to be measured).
pub const TAIL_TEXELS: u32 = 128;

/// The window over which a texture's wanted levels count, seconds (#385's measure).
pub const WINDOW_SECONDS: f64 = 2.0;

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
}

impl Tracked {
    /// Its target at `now`: the finest level wanted within the window, else the level wanted
    /// last if within it, else its tail.
    fn target(&self, now: f64, window: f64) -> usize {
        let within = |w: Option<(usize, f64)>| w.filter(|&(_, at)| now - at <= window);
        within(self.finest)
            .or(within(self.latest))
            .map_or(self.tail, |(level, _)| level.min(self.tail))
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
        }
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
        });
        self.textures.len() - 1
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
    /// counts it in the image it samples.
    pub fn wanted_in_image(&mut self, texture: usize, level: usize, now: f64) {
        if let Some(top) = self.textures.get(texture).map(|t| t.top) {
            self.wanted(texture, level + top, now);
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
        // Within the bytes this plan may upload: the first change always is.
        let fits = |spent: u64, cost: u64, changes: &[LevelChange]| {
            changes.is_empty() || spent + cost <= upload
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
            .filter(|&v| self.textures[v].top < self.textures[v].tail && !changed[v])
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
                    if t.last_use() >= mine {
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
        p.plan(0.0, u64::MAX);
        p.wanted(t, 0, 0.1);
        p.plan(0.1, u64::MAX);
        assert_eq!(p.top(t), 0);
        p.wanted_in_image(t, 2, 0.2);
        // Still within the window of the want of level 0.
        assert!(p.plan(0.2, u64::MAX).is_empty());
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

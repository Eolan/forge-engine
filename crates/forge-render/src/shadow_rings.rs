//! The sun's shadow rings (D-108, step 1): shadow without rays.
//!
//! Five square maps round the camera, each four times wider than the one inside it, 2048 texels
//! a side, on the light's plane. Each ring is a cache of 8 × 8 tiles of the world as the sun sees
//! it: a tile of the image holds the key of the world tile it was drawn from, and a frame draws
//! the tiles whose key is not the one its place in the ring asks for. The ring's place is a pure
//! function of where the camera is now, so nothing here remembers where the camera was.
//!
//! This file is the part that needs no GPU: which tiles to draw, which rectangles that takes, and
//! the matrices of each. [`crate::MeshletRenderer`] records the passes.
//!
//! - **The static layer** holds the casters that stay. Its tiles wrap: a world tile keeps its
//!   place in the image while the ring moves over it.
//! - **The composed layer** is what the shading samples: the static layer copied unwrapped, with
//!   the movers drawn over it.

use glam::{DVec3, Mat4};

/// Rings, finest first.
pub const RING_COUNT: usize = 5;
/// Texels along a side of a ring's map.
pub const RING_TEXELS: u32 = 2048;
/// Tiles along a side of a ring.
pub const RING_TILES: i64 = 8;
/// Texels along a side of a tile.
pub const TILE_TEXELS: u32 = RING_TEXELS / RING_TILES as u32;
/// The finest ring's side, metres; each next one is four times wider.
pub const FINEST_RING_METRES: f64 = 16.0;
/// Half the depth a ring holds along the sun, metres.
pub const DEPTH_HALF_RANGE: f64 = 4096.0;
/// A ring's depth range sits on multiples of this along the sun, metres, so that it stays put
/// while the camera moves; the ring is drawn again when it has to change.
pub const DEPTH_STEP: f64 = 1024.0;
/// The cosine of the angle the sun may turn from a ring's before the ring is drawn again (0.1°).
pub const SUN_TURN_COS: f64 = 0.999_998_476_9;
/// Every how many frames a ring's movers are drawn: the far ones less often.
pub const MOVER_PERIOD: [u64; RING_COUNT] = [1, 1, 2, 4, 8];

/// The light's axes: `sun` towards the sun, `right` and `up` across it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunAxes {
    /// Across the sun.
    pub right: DVec3,
    /// Across the sun and `right`.
    pub up: DVec3,
    /// Towards the sun, unit.
    pub sun: DVec3,
}

impl SunAxes {
    /// The axes of a sun in direction `sun` (need not be unit).
    pub fn new(sun: DVec3) -> Self {
        let sun = sun.normalize();
        let helper = if sun.y.abs() < 0.99 {
            DVec3::Y
        } else {
            DVec3::X
        };
        let right = helper.cross(sun).normalize();
        let up = sun.cross(right);
        Self { right, up, sun }
    }
}

/// A rectangle of a ring's tiles: `x`, `y` in the image's tile slots, `w`, `h` tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotRect {
    /// First slot across.
    pub x: u32,
    /// First slot down.
    pub y: u32,
    /// Tiles across.
    pub w: u32,
    /// Tiles down.
    pub h: u32,
}

/// The movers as a frame finds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingMovers {
    /// None in the scene.
    None,
    /// There, standing as they stood last frame.
    Standing,
    /// There, and some have moved.
    Moving,
}

/// One rectangle of the static layer to draw this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingDraw {
    /// The ring.
    pub ring: usize,
    /// Its slots in the image.
    pub rect: SlotRect,
    /// The world tile of its first slot, on the light's plane.
    pub tile: [i64; 2],
}

/// One copy of the unwrapping, in texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComposeCopy {
    /// Where it starts in the static layer.
    pub from: [u32; 2],
    /// Where it lands in the composed layer.
    pub to: [u32; 2],
    /// Its size.
    pub size: [u32; 2],
}

/// What a frame draws of the rings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RingPlan {
    /// The static layer's rectangles, near rings first.
    pub draws: Vec<RingDraw>,
    /// Per ring, the tiles of its composed layer to lay again from the static one (a bit per
    /// tile, `y * RING_TILES + x`, the window's first tile at 0; all of them when it moved).
    pub compose: [u64; RING_COUNT],
    /// The rings whose movers are drawn over that.
    pub movers: [bool; RING_COUNT],
}

/// One ring's state.
#[derive(Clone, Debug)]
pub struct Ring {
    width: f64,
    axes: Option<SunAxes>,
    /// The middle of the depth it holds, along the sun, metres.
    depth_centre: f64,
    /// Its first world tile on each axis: the ring covers `window .. window + RING_TILES`.
    window: [i64; 2],
    /// Which slot a world tile takes: `(tile − base) mod RING_TILES`.
    base: [i64; 2],
    /// Per slot (`y * RING_TILES + x`), the world tile it holds.
    keys: [Option<[i64; 2]>; (RING_TILES * RING_TILES) as usize],
    /// Per slot, a bit when what it holds has changed in the world.
    stale: u64,
    /// Whether its composed layer has been laid at all.
    composed: bool,
    /// The whole composed layer is to be laid again: the window moved, or the ring was drawn.
    full: bool,
    /// Tiles of the composed layer to lay again, as [`RingPlan::compose`].
    pending: u64,
    /// The tiles the movers were last drawn over.
    mover_tiles: u64,
}

impl Ring {
    fn new(index: usize) -> Self {
        Self {
            width: FINEST_RING_METRES * 4.0_f64.powi(index as i32),
            axes: None,
            depth_centre: 0.0,
            window: [0; 2],
            base: [0; 2],
            keys: [None; (RING_TILES * RING_TILES) as usize],
            stale: 0,
            composed: false,
            full: false,
            pending: 0,
            mover_tiles: 0,
        }
    }

    /// The ring's side, metres.
    pub fn width(&self) -> f64 {
        self.width
    }

    fn tile_metres(&self) -> f64 {
        self.width / RING_TILES as f64
    }

    /// Metres a texel.
    pub fn texel(&self) -> f64 {
        self.width / f64::from(RING_TEXELS)
    }

    /// Whether it has been drawn at all: a ring that has not casts nothing yet.
    pub fn ready(&self) -> bool {
        self.axes.is_some()
    }

    /// Its axes, once drawn.
    pub fn axes(&self) -> Option<SunAxes> {
        self.axes
    }

    /// Its first world tile on each axis.
    pub fn window(&self) -> [i64; 2] {
        self.window
    }

    fn window_for(&self, camera: DVec3, axes: SunAxes) -> [i64; 2] {
        let tile = self.tile_metres();
        [
            (camera.dot(axes.right) / tile).floor() as i64 - RING_TILES / 2,
            (camera.dot(axes.up) / tile).floor() as i64 - RING_TILES / 2,
        ]
    }

    fn depth_centre_for(camera: DVec3, axes: SunAxes) -> f64 {
        (camera.dot(axes.sun) / DEPTH_STEP).round() * DEPTH_STEP
    }

    /// The world tile slot `s` holds on an axis, in the current window.
    fn slot_tile(&self, axis: usize, s: i64) -> i64 {
        self.window[axis] + (self.base[axis] + s - self.window[axis]).rem_euclid(RING_TILES)
    }

    /// The tiles of `mask` (a bit per slot) as rectangles, each contiguous in the world.
    fn rectangles(&self, mut mask: u64) -> Vec<SlotRect> {
        let n = RING_TILES as u32;
        let bit = |x: u32, y: u32| 1u64 << (y * n + x);
        let mut out = Vec::new();
        while mask != 0 {
            let first = mask.trailing_zeros();
            let (x, y) = (first % n, first / n);
            let mut w = 1;
            while x + w < n
                && mask & bit(x + w, y) != 0
                && self.slot_tile(0, i64::from(x + w)) == self.slot_tile(0, i64::from(x)) + i64::from(w)
            {
                w += 1;
            }
            let mut h = 1;
            'rows: while y + h < n
                && self.slot_tile(1, i64::from(y + h)) == self.slot_tile(1, i64::from(y)) + i64::from(h)
            {
                for i in 0..w {
                    if mask & bit(x + i, y + h) == 0 {
                        break 'rows;
                    }
                }
                h += 1;
            }
            for b in 0..h {
                for a in 0..w {
                    mask &= !bit(x + a, y + b);
                }
            }
            out.push(SlotRect { x, y, w, h });
        }
        out
    }

    fn draw_of(&self, ring: usize, rect: SlotRect) -> RingDraw {
        RingDraw {
            ring,
            rect,
            tile: [
                self.slot_tile(0, i64::from(rect.x)),
                self.slot_tile(1, i64::from(rect.y)),
            ],
        }
    }

    /// The slots whose key is not the one this window asks for.
    fn missing(&self) -> u64 {
        let mut mask = 0u64;
        for y in 0..RING_TILES {
            for x in 0..RING_TILES {
                let wanted = [self.slot_tile(0, x), self.slot_tile(1, y)];
                if self.keys[(y * RING_TILES + x) as usize] != Some(wanted) {
                    mask |= 1 << (y * RING_TILES + x);
                }
            }
        }
        mask
    }

    fn settle_keys(&mut self, mask: u64) {
        for y in 0..RING_TILES {
            for x in 0..RING_TILES {
                let at = y * RING_TILES + x;
                if mask & (1 << at) != 0 {
                    self.keys[at as usize] = Some([self.slot_tile(0, x), self.slot_tile(1, y)]);
                    self.stale &= !(1 << at);
                }
            }
        }
    }

    /// Takes a new sun, depth and window, with the whole ring to draw as one rectangle.
    fn start_over(&mut self, ring: usize, axes: SunAxes, depth: f64, window: [i64; 2]) -> RingDraw {
        self.axes = Some(axes);
        self.depth_centre = depth;
        self.window = window;
        self.base = window;
        self.settle_keys(u64::MAX);
        self.full = true;
        self.draw_of(
            ring,
            SlotRect {
                x: 0,
                y: 0,
                w: RING_TILES as u32,
                h: RING_TILES as u32,
            },
        )
    }

    /// The composed layer's tile that slot (`x`, `y`) of the static layer shows.
    fn composed_bit(&self, x: i64, y: i64) -> u64 {
        let (cx, cy) = (self.slot_tile(0, x) - self.window[0], self.slot_tile(1, y) - self.window[1]);
        1 << (cy * RING_TILES + cx)
    }

    /// The tiles of the composed layer that the spheres `movers` cover along the sun.
    fn movers_cover(&self, movers: &[(DVec3, f64)]) -> u64 {
        let Some(axes) = self.axes else {
            return 0;
        };
        let t = self.tile_metres();
        let tile = |x: f64| (x / t).floor() as i64;
        let mut mask = 0u64;
        for &(center, radius) in movers {
            if radius <= 0.0 {
                continue;
            }
            let (u, v) = (center.dot(axes.right), center.dot(axes.up));
            let x0 = (tile(u - radius) - self.window[0]).max(0);
            let x1 = (tile(u + radius) - self.window[0]).min(RING_TILES - 1);
            let y0 = (tile(v - radius) - self.window[1]).max(0);
            let y1 = (tile(v + radius) - self.window[1]).min(RING_TILES - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    mask |= 1 << (y * RING_TILES + x);
                }
            }
        }
        mask
    }

    /// The copies that lay `tiles` of the composed layer from the wrapped static layer, the
    /// window's first world tile at the image's corner.
    pub fn compose_copies(&self, tiles: u64) -> Vec<ComposeCopy> {
        if tiles != u64::MAX {
            let mut out = Vec::new();
            for cy in 0..RING_TILES {
                for cx in 0..RING_TILES {
                    if tiles & (1 << (cy * RING_TILES + cx)) == 0 {
                        continue;
                    }
                    let sx = (self.window[0] + cx - self.base[0]).rem_euclid(RING_TILES) as u32;
                    let sy = (self.window[1] + cy - self.base[1]).rem_euclid(RING_TILES) as u32;
                    out.push(ComposeCopy {
                        from: [sx * TILE_TEXELS, sy * TILE_TEXELS],
                        to: [cx as u32 * TILE_TEXELS, cy as u32 * TILE_TEXELS],
                        size: [TILE_TEXELS, TILE_TEXELS],
                    });
                }
            }
            return out;
        }
        let split = |axis: usize| -> Vec<(u32, u32, u32)> {
            let first = (self.window[axis] - self.base[axis]).rem_euclid(RING_TILES) as u32;
            let n = RING_TILES as u32;
            if first == 0 {
                vec![(0, 0, n)]
            } else {
                vec![(first, 0, n - first), (0, n - first, first)]
            }
        };
        let mut out = Vec::new();
        for &(sy, dy, h) in &split(1) {
            for &(sx, dx, w) in &split(0) {
                out.push(ComposeCopy {
                    from: [sx * TILE_TEXELS, sy * TILE_TEXELS],
                    to: [dx * TILE_TEXELS, dy * TILE_TEXELS],
                    size: [w * TILE_TEXELS, h * TILE_TEXELS],
                });
            }
        }
        out
    }

    /// From a point relative to `camera` to the clip space of `tiles` of this ring (first world
    /// tile `tile`): x and y over the rectangle, z the height along the sun in the ring's depth
    /// range, greater towards the sun.
    pub fn clip_from_relative(&self, camera: DVec3, tile: [i64; 2], tiles: [u32; 2]) -> Mat4 {
        let axes = self.axes.expect("a ring is drawn after it has its axes");
        let t = self.tile_metres();
        let (w, h) = (f64::from(tiles[0]) * t, f64::from(tiles[1]) * t);
        let u = camera.dot(axes.right) - tile[0] as f64 * t;
        let v = camera.dot(axes.up) - tile[1] as f64 * t;
        let range = 2.0 * DEPTH_HALF_RANGE;
        let z = (camera.dot(axes.sun) - (self.depth_centre - DEPTH_HALF_RANGE)) / range;
        Mat4::from_cols_array(&[
            (axes.right.x * 2.0 / w) as f32,
            (axes.up.x * 2.0 / h) as f32,
            (axes.sun.x / range) as f32,
            0.0,
            (axes.right.y * 2.0 / w) as f32,
            (axes.up.y * 2.0 / h) as f32,
            (axes.sun.y / range) as f32,
            0.0,
            (axes.right.z * 2.0 / w) as f32,
            (axes.up.z * 2.0 / h) as f32,
            (axes.sun.z / range) as f32,
            0.0,
            (u * 2.0 / w - 1.0) as f32,
            (v * 2.0 / h - 1.0) as f32,
            z as f32,
            1.0,
        ])
    }

    /// The side planes of `tiles` of this ring, relative to `camera`: what can cast into that
    /// rectangle along the sun lies between them. The last two never reject.
    pub fn planes(&self, camera: DVec3, tile: [i64; 2], tiles: [u32; 2]) -> [[f32; 4]; 6] {
        let axes = self.axes.expect("a ring is drawn after it has its axes");
        let t = self.tile_metres();
        let (w, h) = (f64::from(tiles[0]) * t, f64::from(tiles[1]) * t);
        let u = camera.dot(axes.right) - tile[0] as f64 * t;
        let v = camera.dot(axes.up) - tile[1] as f64 * t;
        let plane = |n: DVec3, d: f64| [n.x as f32, n.y as f32, n.z as f32, d as f32];
        [
            plane(axes.right, u),
            plane(-axes.right, w - u),
            plane(axes.up, v),
            plane(-axes.up, h - v),
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    /// What the shading reads of this ring in a frame whose camera is at `camera`.
    pub fn sampling(&self, camera: DVec3, image: u32) -> GpuShadowRing {
        let Some(axes) = self.axes.filter(|_| self.composed) else {
            return GpuShadowRing::none();
        };
        let t = self.tile_metres();
        let range = 2.0 * DEPTH_HALF_RANGE;
        let axis = |a: DVec3, w: f64| [a.x as f32, a.y as f32, a.z as f32, w as f32];
        GpuShadowRing {
            axis_u: axis(axes.right, camera.dot(axes.right) - self.window[0] as f64 * t),
            axis_v: axis(axes.up, camera.dot(axes.up) - self.window[1] as f64 * t),
            axis_s: axis(
                axes.sun,
                camera.dot(axes.sun) - (self.depth_centre - DEPTH_HALF_RANGE),
            ),
            texel: self.texel() as f32,
            inv_width: (1.0 / self.width) as f32,
            inv_range: (1.0 / range) as f32,
            image,
        }
    }
}

/// Mirrors `ShadowRing` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuShadowRing {
    /// `right`, and what to add to a relative point's dot with it to get metres across the ring.
    pub axis_u: [f32; 4],
    /// The same for `up`.
    pub axis_v: [f32; 4],
    /// The same for `sun`, from the bottom of the depth range.
    pub axis_s: [f32; 4],
    /// Metres a texel.
    pub texel: f32,
    /// 1 / the ring's side.
    pub inv_width: f32,
    /// 1 / the depth range.
    pub inv_range: f32,
    /// Sampled index of its composed layer, or `u32::MAX`: not ready.
    pub image: u32,
}

impl GpuShadowRing {
    /// A ring not to read.
    pub fn none() -> Self {
        Self {
            axis_u: [0.0; 4],
            axis_v: [0.0; 4],
            axis_s: [0.0; 4],
            texel: 1.0,
            inv_width: 1.0,
            inv_range: 1.0,
            image: u32::MAX,
        }
    }
}

/// The filter's settings (`r.shadows.*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingFilter {
    /// Taps of the blocker search.
    pub search_taps: u32,
    /// Taps of the filter.
    pub filter_taps: u32,
    /// The widest filter's radius, texels.
    pub max_radius: f32,
    /// How far the search looks along the sun for the widest penumbra, metres.
    pub search_reach: f32,
    /// Depth bias, texels.
    pub bias: f32,
    /// Normal offset, texels.
    pub normal_offset: f32,
}

impl Default for RingFilter {
    fn default() -> Self {
        Self {
            search_taps: 12,
            filter_taps: 12,
            max_radius: 24.0,
            search_reach: 64.0,
            bias: 1.5,
            normal_offset: 2.0,
        }
    }
}

/// Mirrors `RingSet` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuRingSet {
    /// The rings, finest first.
    pub rings: [GpuShadowRing; RING_COUNT],
    /// The tangent of the sun's angular radius.
    pub tan_radius: f32,
    /// See [`RingFilter`].
    pub search_taps: u32,
    /// See [`RingFilter`].
    pub filter_taps: u32,
    /// See [`RingFilter`].
    pub max_radius: f32,
    /// See [`RingFilter`].
    pub search_reach: f32,
    /// See [`RingFilter`].
    pub bias: f32,
    /// See [`RingFilter`].
    pub normal_offset: f32,
    /// Keeps the block on 16 bytes.
    pub pad: f32,
}

/// The rings' caches.
#[derive(Clone, Debug)]
pub struct RingCache {
    rings: Vec<Ring>,
}

impl Default for RingCache {
    fn default() -> Self {
        Self::new()
    }
}

impl RingCache {
    /// Rings not drawn yet.
    pub fn new() -> Self {
        Self {
            rings: (0..RING_COUNT).map(Ring::new).collect(),
        }
    }

    /// Ring `index`.
    pub fn ring(&self, index: usize) -> &Ring {
        &self.rings[index]
    }

    /// Forgets everything: the next frame draws every ring.
    pub fn clear(&mut self) {
        *self = Self::new();
    }

    /// Marks what the world changed: the tiles the sphere covers along the sun, of every ring.
    /// What the sun sees of a caster lies under its own footprint on the light's plane, so
    /// nothing else can have changed. A marked tile keeps its picture until its turn.
    pub fn mark_changed(&mut self, center: DVec3, radius: f64) {
        for ring in &mut self.rings {
            let Some(axes) = ring.axes else {
                continue;
            };
            let t = ring.tile_metres();
            let (u, v) = (center.dot(axes.right), center.dot(axes.up));
            let tile = |x: f64| (x / t).floor() as i64;
            for ty in tile(v - radius)..=tile(v + radius) {
                for tx in tile(u - radius)..=tile(u + radius) {
                    let (dx, dy) = (tx - ring.window[0], ty - ring.window[1]);
                    if (0..RING_TILES).contains(&dx) && (0..RING_TILES).contains(&dy) {
                        let slot_x = (tx - ring.base[0]).rem_euclid(RING_TILES);
                        let slot_y = (ty - ring.base[1]).rem_euclid(RING_TILES);
                        ring.stale |= 1 << (slot_y * RING_TILES + slot_x);
                    }
                }
            }
        }
    }

    /// This frame's work: which rectangles of the static layer to draw (at most `budget` for
    /// moves and changes, more for a ring that has nothing at all to show yet), what to compose
    /// and over which rings to draw the movers, whose bounding spheres are `mover_bounds`.
    pub fn plan(
        &mut self,
        camera: DVec3,
        sun: DVec3,
        budget: usize,
        movers: RingMovers,
        mover_bounds: &[(DVec3, f64)],
        frame: u64,
    ) -> RingPlan {
        let now = SunAxes::new(sun);
        let mut plan = RingPlan::default();
        let mut left = budget;
        // One ring a frame takes a new sun or depth range: near ones first.
        let mut redraw_left = true;
        for (index, ring) in self.rings.iter_mut().enumerate() {
            let Some(axes) = ring.axes else {
                let window = ring.window_for(camera, now);
                let depth = Ring::depth_centre_for(camera, now);
                plan.draws.push(ring.start_over(index, now, depth, window));
                continue;
            };
            let turned = axes.sun.dot(now.sun) < SUN_TURN_COS;
            let depth = Ring::depth_centre_for(camera, axes);
            if (turned || depth != ring.depth_centre) && redraw_left {
                redraw_left = false;
                left = left.saturating_sub(1);
                let depth = Ring::depth_centre_for(camera, now);
                let window = ring.window_for(camera, now);
                plan.draws.push(ring.start_over(index, now, depth, window));
                continue;
            }
            let wanted = ring.window_for(camera, axes);
            if wanted == ring.window {
                continue;
            }
            let overlaps = (wanted[0] - ring.window[0]).abs() < RING_TILES
                && (wanted[1] - ring.window[1]).abs() < RING_TILES;
            if !overlaps {
                // Nothing of it is of use where the camera now is.
                plan.draws
                    .push(ring.start_over(index, axes, ring.depth_centre, wanted));
                continue;
            }
            let before = ring.window;
            ring.window = wanted;
            let missing = ring.missing();
            let rects = ring.rectangles(missing);
            if rects.len() > left {
                // Stays where it is, still whole, until it can move in one frame.
                ring.window = before;
                continue;
            }
            left -= rects.len();
            for rect in rects {
                let draw = ring.draw_of(index, rect);
                plan.draws.push(draw);
            }
            ring.settle_keys(missing);
            ring.full = true;
        }
        // What changed in the world, as the budget allows.
        for (index, ring) in self.rings.iter_mut().enumerate() {
            if ring.stale == 0 || left == 0 {
                continue;
            }
            let rects = ring.rectangles(ring.stale);
            for rect in rects.into_iter().take(left) {
                left -= 1;
                let draw = ring.draw_of(index, rect);
                plan.draws.push(draw);
                let mut mask = 0u64;
                for y in rect.y..rect.y + rect.h {
                    for x in rect.x..rect.x + rect.w {
                        mask |= 1 << (y * RING_TILES as u32 + x);
                        ring.pending |= ring.composed_bit(i64::from(x), i64::from(y));
                    }
                }
                ring.stale &= !mask;
            }
        }
        for (index, ring) in self.rings.iter_mut().enumerate() {
            let present = movers != RingMovers::None;
            let due = movers == RingMovers::Moving && frame % MOVER_PERIOD[index] == 0;
            let whole = ring.full || !ring.composed;
            let gone = !present && ring.mover_tiles != 0;
            if !(whole || due || gone || ring.pending != 0) {
                continue;
            }
            // Laying a tile lifts the movers off it, so a ring laid at all has them drawn again,
            // over every tile that held them and every tile that will.
            let now = if present {
                ring.movers_cover(mover_bounds)
            } else {
                0
            };
            let tiles = if whole {
                u64::MAX
            } else {
                ring.pending | ring.mover_tiles | now
            };
            plan.compose[index] = tiles;
            plan.movers[index] = now != 0;
            ring.mover_tiles = now;
            ring.pending = 0;
            ring.full = false;
            ring.composed = true;
        }
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOVER: [(DVec3, f64); 1] = [(DVec3::ZERO, 0.5)];

    fn sun() -> DVec3 {
        DVec3::new(0.3, 0.8, 0.5)
    }

    fn across(axes: SunAxes, tiles: f64, ring: &Ring) -> DVec3 {
        axes.right * tiles * ring.tile_metres()
    }

    #[test]
    fn the_first_frame_draws_each_ring_whole() {
        let mut c = RingCache::new();
        let plan = c.plan(DVec3::ZERO, sun(), 2, RingMovers::None, &[], 0);
        assert_eq!(plan.draws.len(), RING_COUNT);
        for d in &plan.draws {
            assert_eq!(
                d.rect,
                SlotRect {
                    x: 0,
                    y: 0,
                    w: 8,
                    h: 8
                }
            );
        }
        assert_eq!(plan.compose, [u64::MAX; RING_COUNT]);
    }

    #[test]
    fn a_camera_standing_still_draws_nothing() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 0);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 1);
        assert!(plan.draws.is_empty());
        assert_eq!(plan.compose, [0; RING_COUNT]);
    }

    #[test]
    fn a_tile_along_draws_one_column_of_the_finest_ring() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 0);
        let axes = c.ring(0).axes().unwrap();
        let step = across(axes, 1.0, c.ring(0));
        let plan = c.plan(step, sun(), 8, RingMovers::None, &[], 1);
        assert_eq!(plan.draws.len(), 1);
        let d = plan.draws[0];
        assert_eq!(d.ring, 0);
        assert_eq!((d.rect.w, d.rect.h), (1, 8));
        // The column leaving is the one the new tiles take.
        assert_eq!(d.rect.x, 0);
    }

    #[test]
    fn where_a_ring_stands_depends_only_on_where_the_camera_is() {
        // Two paths to one place: the same window, tile for tile.
        let (mut a, mut b) = (RingCache::new(), RingCache::new());
        a.plan(DVec3::ZERO, sun(), 64, RingMovers::None, &[], 0);
        b.plan(DVec3::ZERO, sun(), 64, RingMovers::None, &[], 0);
        let axes = a.ring(0).axes().unwrap();
        let end = across(axes, 3.25, a.ring(0));
        a.plan(end, sun(), 64, RingMovers::None, &[], 1);
        for k in 1..=13 {
            b.plan(end * f64::from(k) / 13.0, sun(), 64, RingMovers::None, &[], k as u64);
        }
        for i in 0..RING_COUNT {
            assert_eq!(a.ring(i).window, b.ring(i).window);
        }
    }

    #[test]
    fn every_tile_in_a_window_is_held_after_a_walk() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 64, RingMovers::None, &[], 0);
        let axes = c.ring(0).axes().unwrap();
        for k in 1..300 {
            let at = axes.right * f64::from(k) * 0.37 + axes.up * f64::from(k) * 0.21;
            c.plan(at, sun(), 64, RingMovers::None, &[], k as u64);
            assert_eq!(c.ring(0).missing(), 0, "after step {k}");
            assert_eq!(c.ring(4).missing(), 0, "after step {k}");
        }
    }

    #[test]
    fn a_rectangle_never_runs_across_the_seam() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 64, RingMovers::None, &[], 0);
        let axes = c.ring(0).axes().unwrap();
        let t = c.ring(0).tile_metres();
        c.plan(axes.right * 3.0 * t + axes.up * 2.0 * t, sun(), 64, RingMovers::None, &[], 1);
        // All the slots: four rectangles at most, none crossing where the world tiles jump.
        let r = c.ring(0).rectangles(u64::MAX);
        assert_eq!(r.len(), 4);
        for rect in r {
            for s in 1..rect.w {
                assert_eq!(
                    c.ring(0).slot_tile(0, i64::from(rect.x + s)),
                    c.ring(0).slot_tile(0, i64::from(rect.x)) + i64::from(s)
                );
            }
        }
    }

    #[test]
    fn the_copies_cover_the_composed_layer_once() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 64, RingMovers::None, &[], 0);
        let axes = c.ring(0).axes().unwrap();
        let t = c.ring(0).tile_metres();
        c.plan(axes.right * 5.5 * t + axes.up * 3.5 * t, sun(), 64, RingMovers::None, &[], 1);
        let copies = c.ring(0).compose_copies(u64::MAX);
        assert_eq!(copies.len(), 4);
        let area: u32 = copies.iter().map(|c| c.size[0] * c.size[1]).sum();
        assert_eq!(area, RING_TEXELS * RING_TEXELS);
        // The window's first tile lands in the corner.
        let corner = copies.iter().find(|c| c.to == [0, 0]).unwrap();
        assert_eq!(
            corner.from,
            [5 * TILE_TEXELS, 3 * TILE_TEXELS],
            "the wrap's offset"
        );
    }

    #[test]
    fn a_tight_budget_holds_a_ring_back_whole() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 64, RingMovers::None, &[], 0);
        let axes = c.ring(0).axes().unwrap();
        let before = c.ring(0).window;
        let plan = c.plan(across(axes, 1.0, c.ring(0)), sun(), 0, RingMovers::None, &[], 1);
        assert!(plan.draws.is_empty());
        assert_eq!(c.ring(0).window, before);
        assert_eq!(c.ring(0).missing(), 0);
    }

    #[test]
    fn a_far_jump_draws_the_ring_again_whatever_the_budget() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 0);
        let axes = c.ring(0).axes().unwrap();
        let plan = c.plan(axes.right * 500.0, sun(), 0, RingMovers::None, &[], 1);
        assert!(plan.draws.iter().any(|d| d.ring == 0 && d.rect.w == 8));
        assert_eq!(c.ring(0).missing(), 0);
    }

    #[test]
    fn a_sun_that_turned_redraws_one_ring_a_frame() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 0);
        let turned = DVec3::new(0.4, 0.8, 0.45);
        let plan = c.plan(DVec3::ZERO, turned, 8, RingMovers::None, &[], 1);
        assert_eq!(plan.draws.len(), 1);
        assert_eq!(plan.draws[0].ring, 0);
        let plan = c.plan(DVec3::ZERO, turned, 8, RingMovers::None, &[], 2);
        assert_eq!(plan.draws[0].ring, 1);
        for f in 3..6 {
            assert_eq!(c.plan(DVec3::ZERO, turned, 8, RingMovers::None, &[], f).draws.len(), 1);
        }
        assert!(c.plan(DVec3::ZERO, turned, 8, RingMovers::None, &[], 6).draws.is_empty());
    }

    #[test]
    fn a_sun_barely_moved_redraws_nothing() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 0);
        let barely = sun().normalize() + DVec3::new(1e-5, 0.0, 0.0);
        assert!(c.plan(DVec3::ZERO, barely, 8, RingMovers::None, &[], 1).draws.is_empty());
    }

    #[test]
    fn what_changed_is_drawn_again_under_its_footprint() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 0);
        c.mark_changed(DVec3::ZERO, 0.1);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 1);
        // One small rectangle in each ring, near rings first.
        let rings: Vec<_> = plan.draws.iter().map(|d| d.ring).collect();
        assert_eq!(rings, vec![0, 1, 2, 3, 4]);
        for d in &plan.draws {
            assert!(d.rect.w <= 2 && d.rect.h <= 2);
        }
        assert!(c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 2).draws.is_empty());
    }

    #[test]
    fn the_movers_of_far_rings_come_less_often() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 0);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 1);
        assert_eq!(plan.movers, [true, true, false, false, false]);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 2);
        assert_eq!(plan.movers, [true, true, true, false, false]);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 8);
        assert_eq!(plan.movers, [true; RING_COUNT]);
    }

    #[test]
    fn only_the_tiles_a_mover_touches_are_laid_again() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 0);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 1);
        // Round the camera: the middle of the window, a few tiles of the 64.
        assert!(plan.compose[0] != 0 && plan.compose[0].count_ones() <= 4);
        let copies = c.ring(0).compose_copies(plan.compose[0]);
        assert_eq!(copies.len() as u32, plan.compose[0].count_ones());
        for copy in copies {
            assert_eq!(copy.size, [TILE_TEXELS, TILE_TEXELS]);
        }
    }

    #[test]
    fn a_mover_leaving_a_tile_lays_that_tile_again() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 0);
        let axes = c.ring(0).axes().unwrap();
        let away = [(axes.right * 5.0, 0.5)];
        let first = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 1).compose[0];
        let then = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &away, 2).compose[0];
        assert_eq!(then & first, first, "where it stood");
        assert!(then.count_ones() > first.count_ones(), "and where it stands");
    }

    #[test]
    fn standing_movers_cost_nothing_once_drawn() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 0);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::Standing, &MOVER, 1);
        assert_eq!(plan.compose, [0; RING_COUNT]);
        assert_eq!(plan.movers, [false; RING_COUNT]);
    }

    #[test]
    fn movers_gone_compose_the_rings_once_more() {
        let mut c = RingCache::new();
        c.plan(DVec3::ZERO, sun(), 8, RingMovers::Moving, &MOVER, 0);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 1);
        assert!(plan.compose.iter().all(|t| *t != 0));
        assert_eq!(plan.movers, [false; RING_COUNT]);
        let plan = c.plan(DVec3::ZERO, sun(), 8, RingMovers::None, &[], 2);
        assert_eq!(plan.compose, [0; RING_COUNT]);
    }

    #[test]
    fn a_point_lands_where_the_matrix_says_it_does() {
        let mut c = RingCache::new();
        c.plan(DVec3::new(100.0, 5.0, -40.0), sun(), 8, RingMovers::None, &[], 0);
        let camera = DVec3::new(100.0, 5.0, -40.0);
        let ring = c.ring(1);
        let axes = ring.axes().unwrap();
        let t = ring.tile_metres();
        // The window's first corner, a little above the camera's height along the sun.
        let corner = axes.right * ring.window[0] as f64 * t
            + axes.up * ring.window[1] as f64 * t
            + axes.sun * camera.dot(axes.sun);
        let m = ring.clip_from_relative(camera, ring.window, [8, 8]);
        let clip = m.project_point3((corner - camera).as_vec3());
        assert!((clip.x + 1.0).abs() < 1e-3 && (clip.y + 1.0).abs() < 1e-3);
        assert!(clip.z > 0.0 && clip.z < 1.0);
        // And in the sampling's terms: the corner is (0, 0).
        let s = ring.sampling(camera, 7);
        let rel = (corner - camera).as_vec3();
        let u = (glam::Vec3::from_slice(&s.axis_u).dot(rel) + s.axis_u[3]) * s.inv_width;
        let v = (glam::Vec3::from_slice(&s.axis_v).dot(rel) + s.axis_v[3]) * s.inv_width;
        assert!(u.abs() < 1e-3 && v.abs() < 1e-3);
    }

    #[test]
    fn the_planes_hold_the_rectangle() {
        let camera = DVec3::new(10.0, 2.0, 3.0);
        let mut c = RingCache::new();
        c.plan(camera, sun(), 8, RingMovers::None, &[], 0);
        let ring = c.ring(0);
        let axes = ring.axes().unwrap();
        let p = ring.planes(camera, ring.window, [8, 8]);
        let t = ring.tile_metres();
        let middle = axes.right * (ring.window[0] as f64 + 4.0) * t
            + axes.up * (ring.window[1] as f64 + 4.0) * t;
        let rel = (middle - camera).as_vec3();
        // Any height along the sun.
        for height in [-3000.0, 0.0, 3000.0_f32] {
            let at = rel + axes.sun.as_vec3() * height;
            for plane in p {
                assert!(glam::Vec3::from_slice(&plane).dot(at) + plane[3] >= 0.0);
            }
        }
        let outside = rel + axes.right.as_vec3() * (5.0 * t as f32);
        assert!(
            p.iter()
                .any(|plane| glam::Vec3::from_slice(plane).dot(outside) + plane[3] < 0.0)
        );
    }

    #[test]
    fn nothing_samples_a_ring_not_composed() {
        let c = RingCache::new();
        assert_eq!(c.ring(0).sampling(DVec3::ZERO, 3).image, u32::MAX);
    }
}

//! A first-person walker in a scene of convex pieces (#290): the shared character
//! ([`crate::character::Player`], Jolt's `CharacterVirtual`) in a world of its own that holds
//! the pieces within reach of its feet, kept as it goes, on a ground that may have holes.
//!
//! - **The pieces** ([`Pieces`]) are anything it walks on or against: walls, floors, stair
//!   steps, partitions, guards, furniture, each a convex piece given by its corners. Floors are
//!   pieces like the rest, so a scene has as many storeys as it builds, below the ground too.
//! - **They are kept by their outline:** each piece's footprint goes into every cell it
//!   overlaps, and those whose footprint lies within [`WalkSettings::reach`] of the feet become
//!   fixed bodies, dropped past 1.3 times it. A long wall is kept while any part of it is near.
//! - **They change at run time** (#324): each keeps its id while it is enabled, disabled (a
//!   door's leaf), replaced or removed, and pieces are added. Every walker follows at its next
//!   step, a few bodies rather than a rebuild.
//! - **The ground** ([`Ground`]) is a slab, whole or with rectangular holes where flights go
//!   below it, kept whole from the start.
//!
//! The city plan's streets and houses walk it (D-059's step 4), and scenes outside the
//! repository can too.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use forge_nav::{CrossingMotion, CrossingSpec, Passage, Passages};
use forge_physics::{BodyDesc, BodyId, CharacterDesc, CharacterId, Shape, World, WorldDesc};
use forge_sim::TICK;
use glam::{DVec3, Vec2, Vec3};

use crate::character::Player;

/// Its eyes over its feet, metres.
pub const EYES: f32 = 1.65;
/// Ticks run at most in a frame; a longer frame's rest is dropped.
const CATCH_UP: u32 = 4;
/// How far the ground runs on round its area, metres.
const GROUND_EXTENT: f32 = 20_000.0;
/// A piece over more cells than this is kept from the start rather than by its cells.
const MOST_CELLS: i64 = 400;
/// The pieces' edits a walker follows in a step without optimizing its broad phase (Jolt adds
/// the bodies to it either way; the optimization is for many at once).
const EDITS_IN_PLACE: usize = 16;
/// The walker's radius, metres (the character's).
const RADIUS: f32 = 0.3;
/// How high the floor the walker leaves may lie from its feet for a passage to be met, metres
/// (the character's step).
const STEP: f32 = 0.4;
/// How far ahead of the feet along the walk a passage is met, metres: the walker's radius, half
/// a thick wall and a margin.
pub const MEET_AHEAD: f32 = 0.7;
/// The least walk that crosses, m/s.
pub const CROSS_WALK: f32 = 0.5;
/// How square to a passage a walk must head to cross it: the least cosine of its angle to the
/// passage's across (0.7, about 45°).
const CROSS_SQUARE: f32 = 0.7;
/// How far over the exit's floor the body is tested for room, metres (its feet touch the floor).
const FIT_LIFT: f32 = 0.05;

/// How a walker keeps its pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkSettings {
    /// The cells the pieces are found by, metres a side.
    pub cell: f32,
    /// How far from the feet a piece's footprint is kept, metres; dropped past 1.3 times it.
    pub reach: f32,
    /// The pieces' bodies' convex radius, metres (Jolt's rounding of their edges).
    pub convex_radius: f32,
}

impl Default for WalkSettings {
    /// The city plan's: cells of 50 m, pieces within 80 m, edges rounded by 5 cm.
    fn default() -> Self {
        Self {
            cell: 50.0,
            reach: 80.0,
            convex_radius: 0.05,
        }
    }
}

/// Each piece's hull by its id: its shape's version, the convex radius, the shape (#340).
type Hulls = Mutex<HashMap<u32, (u32, u32, Arc<Shape>)>>;

/// The pieces to walk on and against, found by the cells their footprints overlap.
///
/// Each piece keeps its id (its index) for good. Pieces change at run time (#324): a door's leaf
/// is disabled while the door is open, a piece is added, moved or removed. Every walker that
/// advances among them follows at its next step: it drops the bodies of pieces disabled, removed
/// or replaced, and builds those enabled or added within its reach.
pub struct Pieces {
    pieces: Vec<Vec<Vec3>>,
    /// Each piece's footprint: its least and greatest x and z.
    bounds: Vec<[Vec2; 2]>,
    /// Whether each piece is walked against: false while disabled, and once removed.
    enabled: Vec<bool>,
    /// Whether each piece was removed.
    removed: Vec<bool>,
    /// Each piece's shape's version, raised when it is replaced or removed: a walker drops a
    /// body built from an older one.
    shapes: Vec<u32>,
    /// The edit each piece last had ([`Pieces::generation`] then), 0 if none since `new`.
    edited: Vec<u64>,
    /// The edits made since `new`.
    generation: u64,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// The pieces over more than [`MOST_CELLS`] cells, kept from the start.
    always: Vec<u32>,
    cell: f32,
    /// Each piece's convex shape, made the first time a walker keeps it and shared by every
    /// walker after (#340: a walker's start among 981 pieces took 22.5 ms, most of it their
    /// hulls): its shape's version, the convex radius it was made with, the shape.
    hulls: Hulls,
}

impl Pieces {
    /// The pieces, each a convex piece given by its corners in the world's metres, found by cells
    /// of `settings.cell`. Their ids are their places in `pieces`, all enabled.
    pub fn new(pieces: Vec<Vec<Vec3>>, settings: &WalkSettings) -> Self {
        let n = pieces.len();
        let mut out = Self {
            bounds: pieces.iter().map(|p| footprint(p)).collect(),
            pieces,
            enabled: vec![true; n],
            removed: vec![false; n],
            shapes: vec![0; n],
            edited: vec![0; n],
            generation: 0,
            cells: HashMap::new(),
            always: Vec::new(),
            cell: settings.cell,
            hulls: Mutex::new(HashMap::new()),
        };
        for i in 0..n {
            out.index(i as u32);
        }
        out
    }

    /// How many ids were given, those of removed pieces included.
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// Whether no id was given.
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// Piece `id`'s corners, the world's metres; none once removed.
    pub fn corners(&self, id: u32) -> &[Vec3] {
        &self.pieces[id as usize]
    }

    /// Whether piece `id` is walked against: enabled and not removed.
    pub fn enabled(&self, id: u32) -> bool {
        self.enabled[id as usize]
    }

    /// Enables or disables piece `id` (a door's leaf shut or open); its cells stay, so either is
    /// a flag and the walkers' next step. A removed piece stays disabled.
    pub fn set_enabled(&mut self, id: u32, on: bool) {
        let i = id as usize;
        if self.removed[i] || self.enabled[i] == on {
            return;
        }
        self.enabled[i] = on;
        self.edited_now(id);
    }

    /// Adds a piece, enabled, and gives its id: the next unused one.
    pub fn add(&mut self, corners: Vec<Vec3>) -> u32 {
        let id = self.pieces.len() as u32;
        self.bounds.push(footprint(&corners));
        self.pieces.push(corners);
        self.enabled.push(true);
        self.removed.push(false);
        self.shapes.push(0);
        self.edited.push(0);
        self.index(id);
        self.edited_now(id);
        id
    }

    /// Gives piece `id` new corners (a piece moved or reshaped), keeping its id and whether it is
    /// enabled. A removed piece stays removed.
    pub fn replace(&mut self, id: u32, corners: Vec<Vec3>) {
        let i = id as usize;
        if self.removed[i] {
            return;
        }
        self.unindex(id);
        self.bounds[i] = footprint(&corners);
        self.pieces[i] = corners;
        self.shapes[i] = self.shapes[i].wrapping_add(1);
        self.index(id);
        self.edited_now(id);
    }

    /// Removes piece `id` for good; its id is not given again.
    pub fn remove(&mut self, id: u32) {
        let i = id as usize;
        if self.removed[i] {
            return;
        }
        self.unindex(id);
        self.pieces[i] = Vec::new();
        self.removed[i] = true;
        self.enabled[i] = false;
        self.shapes[i] = self.shapes[i].wrapping_add(1);
        self.edited_now(id);
    }

    /// The edits made since `new`: a walker that saw this many has nothing to catch up.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Piece `id`'s convex shape at `radius`, made once and shared by the walkers (#340); `None`
    /// when its corners make none.
    fn hull(&self, id: u32, radius: f32) -> Option<Arc<Shape>> {
        let version = self.shapes[id as usize];
        let mut hulls = self.hulls.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((v, r, shape)) = hulls.get(&id)
            && *v == version
            && *r == radius.to_bits()
        {
            return Some(Arc::clone(shape));
        }
        let shape = Arc::new(Shape::convex_hull(self.corners(id), radius, 1000.0).ok()?);
        hulls.insert(id, (version, radius.to_bits(), Arc::clone(&shape)));
        Some(shape)
    }

    /// How many enabled pieces the segment from `a` to `b` passes through: what stands between
    /// a sound and its listener (#318). A piece counts as its outline on the ground (the hull of
    /// its corners' x and z) between its lowest and highest corner, which fits upright walls and
    /// level floors; the segment counts it when its height, where it crosses the outline,
    /// overlaps the piece's.
    pub fn between(&self, a: Vec3, b: Vec3) -> usize {
        let (a2, b2) = (Vec2::new(a.x, a.z), Vec2::new(b.x, b.z));
        let reach = 0.5 * (b2 - a2).length() + self.cell;
        self.near(0.5 * (a2 + b2), reach)
            .into_iter()
            .filter(|&i| {
                let piece = &self.pieces[i as usize];
                let Some((t0, t1)) = clip_segment(a2, b2, &hull_xz(piece)) else {
                    return false;
                };
                let (lo, hi) = piece.iter().fold((f32::MAX, f32::MIN), |(lo, hi), c| {
                    (lo.min(c.y), hi.max(c.y))
                });
                let (y0, y1) = (a.y + (b.y - a.y) * t0, a.y + (b.y - a.y) * t1);
                y0.min(y1) <= hi && y0.max(y1) >= lo
            })
            .count()
    }

    /// The pieces enabled whose footprints lie within `r` of `p` (the world's x and z), each
    /// once, in order, the ones kept from the start included.
    pub fn near(&self, p: Vec2, r: f32) -> Vec<u32> {
        let (lo, hi) = (
            cell_of(p - Vec2::splat(r), self.cell),
            cell_of(p + Vec2::splat(r), self.cell),
        );
        let mut out: Vec<u32> = self
            .always
            .iter()
            .copied()
            .filter(|&i| self.enabled(i))
            .collect();
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                if let Some(list) = self.cells.get(&(x, z)) {
                    out.extend(
                        list.iter()
                            .copied()
                            .filter(|&i| self.enabled(i) && self.distance(i, p) <= r),
                    );
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Marks piece `id` edited now.
    fn edited_now(&mut self, id: u32) {
        self.generation += 1;
        self.edited[id as usize] = self.generation;
    }

    /// Piece `id`'s cells, or None for one kept from the start.
    fn cells_of(&self, id: u32) -> Option<((i32, i32), (i32, i32))> {
        let b = self.bounds[id as usize];
        let (lo, hi) = (cell_of(b[0], self.cell), cell_of(b[1], self.cell));
        let count = i64::from(hi.0 - lo.0 + 1) * i64::from(hi.1 - lo.1 + 1);
        (count <= MOST_CELLS).then_some((lo, hi))
    }

    /// Puts piece `id` in its cells, or in those kept from the start.
    fn index(&mut self, id: u32) {
        let Some((lo, hi)) = self.cells_of(id) else {
            self.always.push(id);
            return;
        };
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                self.cells.entry((x, z)).or_default().push(id);
            }
        }
    }

    /// Takes piece `id` out of its cells, or out of those kept from the start.
    fn unindex(&mut self, id: u32) {
        let Some((lo, hi)) = self.cells_of(id) else {
            self.always.retain(|&i| i != id);
            return;
        };
        for x in lo.0..=hi.0 {
            for z in lo.1..=hi.1 {
                if let Some(list) = self.cells.get_mut(&(x, z)) {
                    list.retain(|&i| i != id);
                    if list.is_empty() {
                        self.cells.remove(&(x, z));
                    }
                }
            }
        }
    }

    /// The distance from `p` to piece `i`'s footprint, metres (0 inside it).
    fn distance(&self, i: u32, p: Vec2) -> f32 {
        let [lo, hi] = self.bounds[i as usize];
        (lo - p).max(p - hi).max(Vec2::ZERO).length()
    }
}

/// A piece's footprint: its least and greatest x and z.
fn footprint(piece: &[Vec3]) -> [Vec2; 2] {
    piece.iter().fold(
        [Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)],
        |[lo, hi], p| {
            let q = Vec2::new(p.x, p.z);
            [lo.min(q), hi.max(q)]
        },
    )
}

/// The hull of a piece's corners on the ground (x and z), counter-clockwise (Andrew's monotone
/// chain).
fn hull_xz(piece: &[Vec3]) -> Vec<Vec2> {
    let mut points: Vec<Vec2> = piece.iter().map(|p| Vec2::new(p.x, p.z)).collect();
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut hull: Vec<Vec2> = Vec::with_capacity(points.len() + 1);
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &Vec2>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev())
        };
        for &p in ordered {
            while hull.len() >= start + 2
                && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0
            {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    hull
}

/// The stretch of the segment `a`–`b` inside the convex counter-clockwise polygon `hull`, as
/// the segment's parameters (Cyrus–Beck), if it crosses it.
fn clip_segment(a: Vec2, b: Vec2, hull: &[Vec2]) -> Option<(f32, f32)> {
    if hull.len() < 3 {
        return None;
    }
    let d = b - a;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (i, &p) in hull.iter().enumerate() {
        let q = hull[(i + 1) % hull.len()];
        let e = q - p;
        // Outward for a counter-clockwise polygon.
        let n = Vec2::new(e.y, -e.x);
        let num = n.dot(p - a);
        let den = n.dot(d);
        if den.abs() < 1e-12 {
            if num < 0.0 {
                return None;
            }
        } else if den < 0.0 {
            t0 = t0.max(num / den);
        } else {
            t1 = t1.min(num / den);
        }
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, t1))
}

/// The cell of a point.
fn cell_of(p: Vec2, cell: f32) -> (i32, i32) {
    ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32)
}

/// The ground under the pieces: a slab, whole or with holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Ground {
    /// Its top, metres.
    pub top: f32,
    /// Its thickness, metres. Over a cellar its underside is the cellar's ceiling: a slab thicker
    /// than the gap to the ceiling below stops a capsule's head at the foot of the flight.
    pub thickness: f32,
    /// The rectangle its holes are cut in, its least and greatest x and z; outside it the ground
    /// runs on whole. None: whole everywhere.
    pub area: Option<[Vec2; 2]>,
    /// The holes, each its least and greatest x and z, within `area`: where flights go below it.
    pub holes: Vec<[Vec2; 2]>,
}

impl Ground {
    /// A whole slab, its top at `top`, `thickness` thick.
    pub fn slab(top: f32, thickness: f32) -> Self {
        Self {
            top,
            thickness,
            area: None,
            holes: Vec::new(),
        }
    }

    /// Its rectangles, each its least and greatest x and z: the area less its holes (bands
    /// between the holes' edges, cut where the holes are), then four round the area out to
    /// 20 km; one rectangle 40 km a side without an area.
    pub fn rectangles(&self) -> Vec<[Vec2; 2]> {
        let far = GROUND_EXTENT;
        let Some([lo, hi]) = self.area else {
            return vec![[Vec2::splat(-far), Vec2::splat(far)]];
        };
        let holes: Vec<[Vec2; 2]> = self
            .holes
            .iter()
            .map(|h| [h[0].max(lo), h[1].min(hi)])
            .filter(|h| h[0].x < h[1].x && h[0].y < h[1].y)
            .collect();
        // Bands along z between every hole's edges.
        let mut zs: Vec<f32> = vec![lo.y, hi.y];
        for h in &holes {
            zs.push(h[0].y);
            zs.push(h[1].y);
        }
        zs.sort_by(f32::total_cmp);
        zs.dedup();
        let mut out = Vec::new();
        for band in zs.windows(2) {
            let (z0, z1) = (band[0], band[1]);
            if z1 <= z0 {
                continue;
            }
            // The holes across this band, by x, merged.
            let mut cut: Vec<(f32, f32)> = holes
                .iter()
                .filter(|h| h[0].y < z1 && h[1].y > z0)
                .map(|h| (h[0].x, h[1].x))
                .collect();
            cut.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut x = lo.x;
            for (a, b) in cut {
                if a > x {
                    out.push([Vec2::new(x, z0), Vec2::new(a, z1)]);
                }
                x = x.max(b);
            }
            if x < hi.x {
                out.push([Vec2::new(x, z0), Vec2::new(hi.x, z1)]);
            }
        }
        // Round the area: the sides along z full length, the ends between them.
        out.push([Vec2::new(-far, -far), Vec2::new(lo.x, far)]);
        out.push([Vec2::new(hi.x, -far), Vec2::new(far, far)]);
        out.push([Vec2::new(lo.x, -far), Vec2::new(hi.x, lo.y)]);
        out.push([Vec2::new(lo.x, hi.y), Vec2::new(hi.x, far)]);
        out
    }
}

/// The walker, its world and the pieces it keeps.
pub struct Walker {
    world: World,
    player: Player,
    character: CharacterId,
    /// The pieces it holds: each one's body and the version of the shape it was built from.
    kept: HashMap<u32, (BodyId, u32)>,
    settings: WalkSettings,
    /// The pieces' edits it has followed ([`Pieces::generation`]).
    seen: u64,
    /// Seconds not yet ticked.
    owed: f64,
    /// The crossing under way (#316) and the seconds into it.
    crossing: Option<(CrossingMotion, f32)>,
    /// How the last crossing ended.
    crossed: Option<bool>,
}

impl Walker {
    /// A walker standing at `feet` (the world's metres) among `pieces`, on `ground`.
    pub fn new(
        pieces: &Pieces,
        ground: &Ground,
        feet: Vec3,
        settings: WalkSettings,
    ) -> Result<Self> {
        let mut world = World::new(&WorldDesc::default());
        let half_thick = 0.5 * ground.thickness;
        for [lo, hi] in ground.rectangles() {
            let half = 0.5 * (hi - lo);
            let middle = 0.5 * (hi + lo);
            let slab = Shape::cuboid(Vec3::new(half.x, half_thick, half.y), 0.05, 1000.0)?;
            world.add_body(&BodyDesc::fixed(
                &slab,
                DVec3::new(
                    f64::from(middle.x),
                    f64::from(ground.top) - f64::from(half_thick),
                    f64::from(middle.y),
                ),
            ))?;
        }
        let character = world.add_character(&CharacterDesc {
            position: feet.as_dvec3(),
            ..CharacterDesc::default()
        });
        let mut walker = Self {
            world,
            player: Player {
                character: Some(character),
                ..Player::default()
            },
            character,
            kept: HashMap::new(),
            settings,
            seen: pieces.generation(),
            owed: 0.0,
            crossing: None,
            crossed: None,
        };
        walker.keep(pieces)?;
        tracing::info!(
            at = %format_args!("{:.1},{:.1},{:.1}", feet.x, feet.y, feet.z),
            pieces = walker.kept.len(),
            "the walker (#290)"
        );
        Ok(walker)
    }

    /// Its feet, the world's metres.
    pub fn feet(&self) -> Vec3 {
        self.world.character(self.character).position.as_vec3()
    }

    /// How many pieces it holds as bodies.
    pub fn kept(&self) -> usize {
        self.kept.len()
    }

    /// Walks `walk` (m/s along the ground, the world's x and z) for the ticks `dt` seconds owe,
    /// at most four, then keeps the pieces round its feet. A crossing under way carries the body
    /// instead of the walk.
    pub fn advance(&mut self, pieces: &Pieces, walk: [f32; 2], dt: f64) {
        self.player.walk = walk;
        self.owed += dt;
        let tick = f64::from(TICK);
        let due = (self.owed / tick).floor();
        self.owed -= due * tick;
        for _ in 0..(due as u32).min(CATCH_UP) {
            if self.crossing.is_some() {
                self.carry();
            } else {
                self.player.tick(&mut self.world, None, 0, TICK, |_| None);
            }
        }
        if let Err(e) = self.keep(pieces) {
            tracing::warn!("the walker's pieces: {e}");
        }
    }

    /// [`Walker::advance`], crossing on its own the passage its walk heads into (the owner,
    /// 2026-10-19): when [`Walker::meets`] finds one and `rule` gives a crossing for it (by its
    /// kind and state, never by the body's height: D-076), the body is carried over it.
    pub fn advance_across(
        &mut self,
        pieces: &Pieces,
        passages: &Passages,
        walk: [f32; 2],
        dt: f64,
        rule: impl Fn(&Passage) -> Option<CrossingSpec>,
    ) {
        if self.crossing.is_none()
            && let Some(k) = self.meets(passages, walk)
        {
            let passage = passages.get(k);
            // Only where the body fits past it: walking into a fence with no room behind it does
            // not start a crossing that would only go back.
            if let Some(spec) = rule(passage)
                && let Some(motion) = CrossingMotion::over(passage, self.feet(), RADIUS, spec)
                && self.fits(motion.exit)
            {
                self.cross(motion);
            }
        }
        self.advance(pieces, walk, dt);
    }

    /// The passage its walk heads into: the one the line from its feet along `walk`, [`MEET_AHEAD`]
    /// metres long, meets (#316), the walk at least [`CROSS_WALK`] m/s and within about 45° of
    /// square to it. None while it crosses.
    pub fn meets(&self, passages: &Passages, walk: [f32; 2]) -> Option<usize> {
        let d = Vec3::new(walk[0], 0.0, walk[1]);
        let speed = d.length();
        if speed < CROSS_WALK || self.crossing.is_some() {
            return None;
        }
        let ahead = d / speed;
        let feet = self.feet();
        let (k, _) = passages.met_by(feet, feet + ahead * MEET_AHEAD, STEP)?;
        (ahead.dot(passages.get(k).across).abs() >= CROSS_SQUARE).then_some(k)
    }

    /// Starts carrying the body along `motion` (#316): kinematically, through what it meets,
    /// until its end, where it stays if it fits and goes back to the entry if not
    /// ([`Walker::crossed`]). It faces the way it crosses.
    pub fn cross(&mut self, motion: CrossingMotion) {
        self.player.facing = [motion.ahead.x, motion.ahead.z];
        self.crossing = Some((motion, 0.0));
        self.crossed = None;
    }

    /// The crossing under way, and the seconds into it: the clip's time.
    pub fn crossing(&self) -> Option<(&CrossingMotion, f32)> {
        self.crossing.as_ref().map(|(m, t)| (m, *t))
    }

    /// How the last crossing ended: true when the body fitted at its exit, false when it went
    /// back to its entry; None while one is under way or before the first.
    pub fn crossed(&self) -> Option<bool> {
        self.crossed
    }

    /// Whether the body has room with its feet at `feet`: tested a little over them.
    fn fits(&self, feet: Vec3) -> bool {
        let at = feet + Vec3::Y * FIT_LIFT;
        self.world.character_fits(self.character, at.as_dvec3())
    }

    /// One tick of the crossing under way.
    fn carry(&mut self) {
        let Some((motion, t)) = &mut self.crossing else {
            return;
        };
        *t += TICK;
        if !motion.done(*t) {
            let at = motion.at(*t);
            self.world.place_character(self.character, at.as_dvec3());
            return;
        }
        let (entry, exit) = (motion.entry, motion.exit);
        let fits = self.fits(exit);
        let to = if fits { exit } else { entry };
        self.world.place_character(self.character, to.as_dvec3());
        self.crossing = None;
        self.crossed = Some(fits);
    }

    /// The pieces within reach as bodies, those past 1.3 times it dropped, after following the
    /// pieces' edits since its last step: the bodies of pieces disabled, removed or replaced go
    /// first.
    fn keep(&mut self, pieces: &Pieces) -> Result<()> {
        let feet = self.feet();
        let p = Vec2::new(feet.x, feet.z);
        let reach = self.settings.reach;
        // Bodies brought in or dropped as it walks: the broad phase is optimized after them.
        // Edits alone (a door) leave it as it is unless there are many.
        let mut moved = false;
        let mut edits = 0;
        if pieces.generation() != self.seen {
            let mut stale: Vec<u32> = self
                .kept
                .iter()
                .filter(|&(&i, &(_, shape))| {
                    !pieces.enabled(i) || pieces.shapes[i as usize] != shape
                })
                .map(|(&i, _)| i)
                .collect();
            stale.sort_unstable();
            for i in stale {
                if let Some((body, _)) = self.kept.remove(&i) {
                    self.world.remove_body(body);
                    edits += 1;
                }
            }
        }
        for i in pieces.near(p, reach) {
            if self.kept.contains_key(&i) {
                continue;
            }
            let Some(shape) = pieces.hull(i, self.settings.convex_radius) else {
                continue;
            };
            let body = self.world.add_body(&BodyDesc::fixed(&shape, DVec3::ZERO))?;
            self.kept.insert(i, (body, pieces.shapes[i as usize]));
            if pieces.edited[i as usize] > self.seen {
                edits += 1;
            } else {
                moved = true;
            }
        }
        self.seen = pieces.generation();
        let always = &pieces.always;
        let mut far: Vec<u32> = self
            .kept
            .keys()
            .copied()
            .filter(|&i| !always.contains(&i) && pieces.distance(i, p) > reach * 1.3)
            .collect();
        far.sort_unstable();
        for i in far {
            if let Some((body, _)) = self.kept.remove(&i) {
                self.world.remove_body(body);
                moved = true;
            }
        }
        if moved || edits > EDITS_IN_PLACE {
            self.world.optimize_broad_phase();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_nav::{PassageKind, PassageState};

    /// A box's eight corners.
    fn cuboid(lo: Vec3, hi: Vec3) -> Vec<Vec3> {
        (0..8)
            .map(|k| {
                Vec3::new(
                    if k & 1 == 0 { lo.x } else { hi.x },
                    if k & 2 == 0 { lo.y } else { hi.y },
                    if k & 4 == 0 { lo.z } else { hi.z },
                )
            })
            .collect()
    }

    #[test]
    fn walls_between_count_what_a_sound_passes_through() {
        // Two walls 0.2 m thick across x at z = 2 and z = 5, 3 m high; a floor slab from
        // y = 3 to 3.2 over the whole; a pillar off to the side.
        let pieces = Pieces::new(
            vec![
                cuboid(Vec3::new(-5.0, 0.0, 2.0), Vec3::new(5.0, 3.0, 2.2)),
                cuboid(Vec3::new(-5.0, 0.0, 5.0), Vec3::new(5.0, 3.0, 5.2)),
                cuboid(Vec3::new(-5.0, 3.0, -1.0), Vec3::new(5.0, 3.2, 8.0)),
                cuboid(Vec3::new(8.0, 0.0, 0.0), Vec3::new(9.0, 3.0, 1.0)),
            ],
            &WalkSettings::default(),
        );
        let at = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        assert_eq!(pieces.between(at(0.0, 1.6, 0.0), at(0.0, 1.6, 1.5)), 0);
        assert_eq!(pieces.between(at(0.0, 1.6, 0.0), at(0.0, 1.6, 3.0)), 1);
        assert_eq!(pieces.between(at(0.0, 1.6, 0.0), at(0.0, 1.6, 7.0)), 2);
        // Upstairs, straight above: through the floor only.
        assert_eq!(pieces.between(at(1.0, 1.6, 1.0), at(1.0, 4.5, 1.0)), 1);
        // At 3.1 m, over the walls' tops and inside the floor slab: the floor alone.
        assert_eq!(pieces.between(at(0.0, 3.1, 0.0), at(0.0, 3.1, 7.0)), 1);
        // Beside the pillar.
        assert_eq!(pieces.between(at(7.5, 1.6, -1.0), at(7.5, 1.6, 2.0)), 0);
        // Disabled (a door's leaf open): no longer in the way.
        let mut pieces = pieces;
        pieces.set_enabled(0, false);
        assert_eq!(pieces.between(at(0.0, 1.6, 0.0), at(0.0, 1.6, 3.0)), 0);
    }

    #[test]
    fn a_long_wall_is_near_while_any_of_it_is() {
        // A wall 200 m long along x from 0: its middle 100 m away from a point by its end.
        let wall = cuboid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(200.0, 3.0, 0.2));
        let post = cuboid(Vec3::new(500.0, 0.0, 0.0), Vec3::new(500.2, 1.0, 0.2));
        let pieces = Pieces::new(vec![wall, post], &WalkSettings::default());
        assert_eq!(pieces.near(Vec2::new(-10.0, 5.0), 20.0), vec![0]);
        assert_eq!(pieces.near(Vec2::new(250.0, 0.0), 40.0), Vec::<u32>::new());
        assert_eq!(pieces.near(Vec2::new(250.0, 0.0), 300.0), vec![0, 1]);
        // A piece over more cells than MOST_CELLS is always near.
        let huge = cuboid(
            Vec3::new(-5000.0, 0.0, -5000.0),
            Vec3::new(5000.0, 0.1, 5000.0),
        );
        let pieces = Pieces::new(vec![huge], &WalkSettings::default());
        assert_eq!(pieces.near(Vec2::new(9000.0, 9000.0), 1.0), vec![0]);
    }

    #[test]
    fn pieces_keep_their_ids_through_edits() {
        let settings = WalkSettings::default();
        let a = cuboid(Vec3::ZERO, Vec3::new(1.0, 3.0, 1.0));
        let b = cuboid(Vec3::new(10.0, 0.0, 0.0), Vec3::new(11.0, 3.0, 1.0));
        let mut pieces = Pieces::new(vec![a, b], &settings);
        let near = |pieces: &Pieces| pieces.near(Vec2::ZERO, 20.0);
        assert_eq!(near(&pieces), vec![0, 1]);
        pieces.set_enabled(0, false);
        assert_eq!(near(&pieces), vec![1]);
        pieces.set_enabled(0, true);
        let c = pieces.add(cuboid(Vec3::new(5.0, 0.0, 5.0), Vec3::new(6.0, 3.0, 6.0)));
        assert_eq!(c, 2);
        assert_eq!(near(&pieces), vec![0, 1, 2]);
        // Moved 100 m off: out of the cells round the origin, found where it went.
        pieces.replace(
            1,
            cuboid(Vec3::new(110.0, 0.0, 0.0), Vec3::new(111.0, 3.0, 1.0)),
        );
        assert_eq!(near(&pieces), vec![0, 2]);
        assert_eq!(pieces.near(Vec2::new(110.0, 0.0), 5.0), vec![1]);
        // Removed for good: its id stays taken.
        pieces.remove(0);
        pieces.set_enabled(0, true);
        assert_eq!(near(&pieces), vec![2]);
        assert!(!pieces.enabled(0) && pieces.corners(0).is_empty());
        assert_eq!(pieces.add(cuboid(Vec3::ZERO, Vec3::ONE)), 3);
        // A piece kept from the start (over more than MOST_CELLS cells) as well.
        let huge = pieces.add(cuboid(
            Vec3::new(-5000.0, 0.0, -5000.0),
            Vec3::new(5000.0, 0.1, 5000.0),
        ));
        let far = Vec2::new(9000.0, 9000.0);
        assert_eq!(pieces.near(far, 1.0), vec![huge]);
        pieces.set_enabled(huge, false);
        assert!(pieces.near(far, 1.0).is_empty());
        pieces.remove(huge);
        assert!(pieces.always.is_empty());
        // Every edit counted once: those that changed nothing (enabling an enabled piece, a
        // removed one) not at all.
        assert_eq!(pieces.generation(), 9);
    }

    #[test]
    fn walkers_follow_a_door_shut_opened_moved_and_removed() {
        let settings = WalkSettings::default();
        // A leaf across the way, wider than the walk drifts.
        let leaf = |x: f32| cuboid(Vec3::new(x, 0.0, -20.0), Vec3::new(x + 0.1, 3.0, 20.0));
        let mut pieces = Pieces::new(vec![leaf(5.0)], &settings);
        let ground = Ground::slab(0.0, 1.0);
        let start = Vec3::new(0.0, 0.01, 0.0);
        let mut walker = Walker::new(&pieces, &ground, start, settings).unwrap();
        let walk = |walker: &mut Walker, pieces: &Pieces, seconds: f32| {
            for _ in 0..(seconds / TICK) as u32 {
                walker.advance(pieces, [2.0, 0.0], f64::from(TICK));
            }
            walker.feet().x
        };
        // Shut: it stops at the leaf.
        let x = walk(&mut walker, &pieces, 4.0);
        assert!(x > 4.0 && x < 5.0, "{x}");
        // Open: the leaf disabled, it walks on.
        pieces.set_enabled(0, false);
        let x = walk(&mut walker, &pieces, 2.0);
        assert!(x > 6.0, "{x}");
        assert_eq!(walker.kept(), 0);
        // Shut again, moved ahead of it: stopped there.
        pieces.replace(0, leaf(12.0));
        pieces.set_enabled(0, true);
        let x = walk(&mut walker, &pieces, 6.0);
        assert!(x > 11.0 && x < 12.0, "{x}");
        // Removed, and another added further on: past the first, stopped by the second.
        pieces.remove(0);
        let next = pieces.add(leaf(20.0));
        let x = walk(&mut walker, &pieces, 6.0);
        assert!(x > 19.0 && x < 20.0, "{x}");
        assert_eq!(walker.kept(), 1);
        assert!(walker.kept.contains_key(&next));
        // A walker made after the edits meets the same leaf.
        let mut late = Walker::new(&pieces, &ground, start, settings).unwrap();
        let x = walk(&mut late, &pieces, 12.0);
        assert!(x > 19.0 && x < 20.0, "{x}");
    }

    /// A garden wall 1 m high and 0.2 m thick across the way at x = 5, from z = −2 to 2.
    fn garden_wall() -> Vec<Vec3> {
        cuboid(Vec3::new(5.0, 0.0, -2.0), Vec3::new(5.2, 1.0, 2.0))
    }

    /// The garden wall's passage on its middle line, of `kind` and in `state`.
    fn garden_passage(kind: PassageKind, state: PassageState) -> Passages {
        Passages::new(vec![Passage {
            kind,
            ends: [Vec3::new(5.1, 0.0, -2.0), Vec3::new(5.1, 0.0, 2.0)],
            across: Vec3::X,
            sill: 1.0,
            head: None,
            depth: 0.2,
            drop: 0.0,
            both_ways: true,
            state,
            piece: 0,
        }])
    }

    /// A person's crossing of an open passage, none of a shut one.
    fn open_ones(p: &Passage) -> Option<CrossingSpec> {
        (p.state == PassageState::Open).then(|| CrossingSpec::person(p.crossing()))
    }

    /// Walks +x at 2 m/s for `seconds`, crossing what it may; its feet at each tick.
    fn walk_across(
        walker: &mut Walker,
        pieces: &Pieces,
        passages: &Passages,
        seconds: f32,
    ) -> Vec<Vec3> {
        (0..(seconds / TICK) as u32)
            .map(|_| {
                walker.advance_across(pieces, passages, [2.0, 0.0], f64::from(TICK), open_ones);
                walker.feet()
            })
            .collect()
    }

    #[test]
    fn a_walker_vaults_the_low_wall_it_walks_into_but_not_a_shut_window() {
        let settings = WalkSettings::default();
        let ground = Ground::slab(0.0, 1.0);
        let start = Vec3::new(3.0, 0.01, 0.3);
        let pieces = Pieces::new(vec![garden_wall()], &settings);
        // Open: vaulted, over the wall's top across its thickness, and on past it.
        let open = garden_passage(PassageKind::LowFence, PassageState::Open);
        let mut walker = Walker::new(&pieces, &ground, start, settings).unwrap();
        let feet = walk_across(&mut walker, &pieces, &open, 3.0);
        assert_eq!(walker.crossed(), Some(true));
        let over: Vec<&Vec3> = feet.iter().filter(|f| (f.x - 5.1).abs() <= 0.1).collect();
        assert!(
            !over.is_empty() && over.iter().all(|f| f.y > 1.0),
            "{over:?}"
        );
        assert!(walker.feet().x > 6.5, "{}", walker.feet());
        // Shut window: the rule gives no crossing, the wall stops it.
        let shut = garden_passage(PassageKind::Window, PassageState::Shut);
        let mut walker = Walker::new(&pieces, &ground, start, settings).unwrap();
        walk_across(&mut walker, &pieces, &shut, 3.0);
        assert!(walker.feet().x < 4.8, "{}", walker.feet());
        assert_eq!(walker.crossed(), None);
        // No room past it (a wall 0.25 m behind): not started.
        let behind = cuboid(Vec3::new(5.45, 0.0, -2.0), Vec3::new(5.65, 2.5, 2.0));
        let blocked = Pieces::new(vec![garden_wall(), behind.clone()], &settings);
        let mut walker = Walker::new(&blocked, &ground, start, settings).unwrap();
        walk_across(&mut walker, &blocked, &open, 3.0);
        assert!(walker.feet().x < 4.8, "{}", walker.feet());
        assert_eq!(walker.crossed(), None);
    }

    #[test]
    fn a_crossing_whose_exit_fills_meanwhile_goes_back_to_its_entry() {
        let settings = WalkSettings::default();
        let ground = Ground::slab(0.0, 1.0);
        let mut pieces = Pieces::new(vec![garden_wall()], &settings);
        let open = garden_passage(PassageKind::LowFence, PassageState::Open);
        let mut walker =
            Walker::new(&pieces, &ground, Vec3::new(3.0, 0.01, 0.3), settings).unwrap();
        while walker.crossing().is_none() {
            walker.advance_across(&pieces, &open, [2.0, 0.0], f64::from(TICK), open_ones);
        }
        let entry = walker.crossing().map(|(m, _)| m.entry).unwrap();
        // A crate dropped where it would land.
        pieces.add(cuboid(Vec3::new(5.3, 0.0, -1.0), Vec3::new(6.3, 1.2, 1.0)));
        while walker.crossing().is_some() {
            walker.advance(&pieces, [2.0, 0.0], f64::from(TICK));
        }
        assert_eq!(walker.crossed(), Some(false));
        assert!((walker.feet() - entry).length() < 1e-4, "{}", walker.feet());
    }

    #[test]
    fn a_door_toggled_among_many_pieces_costs_a_step_not_a_rebuild() {
        // A town's worth of pieces round the walker: 40 000 posts on a 1 m grid, half of them
        // within its reach, and a door leaf among them.
        let settings = WalkSettings::default();
        let mut list = Vec::new();
        for i in 0..200 {
            for j in 0..200 {
                let at = Vec3::new(i as f32 - 100.0, 0.0, j as f32 - 100.0);
                list.push(cuboid(at, at + Vec3::new(0.2, 3.0, 0.2)));
            }
        }
        let door = list.len() as u32;
        list.push(cuboid(Vec3::new(0.3, 0.0, 0.05), Vec3::new(0.9, 2.1, 0.15)));
        let ground = Ground::slab(0.0, 1.0);
        let start = Vec3::new(0.6, 0.01, 0.6);
        let t = std::time::Instant::now();
        let mut pieces = Pieces::new(list, &settings);
        let mut walker = Walker::new(&pieces, &ground, start, settings).unwrap();
        let rebuild = t.elapsed();
        let kept = walker.kept();
        let mut toggles = Vec::new();
        for k in 0..6 {
            let t = std::time::Instant::now();
            pieces.set_enabled(door, k % 2 == 1);
            walker.advance(&pieces, [0.0, 0.0], 0.0);
            toggles.push(t.elapsed());
            assert_eq!(walker.kept(), kept - usize::from(k % 2 == 0));
        }
        let toggle = toggles.iter().min().copied().unwrap_or_default();
        let t = std::time::Instant::now();
        walker.advance(&pieces, [0.0, 0.0], 0.0);
        let still = t.elapsed();
        let t = std::time::Instant::now();
        walker.world.optimize_broad_phase();
        let optimize = t.elapsed();
        eprintln!(
            "{kept} pieces kept: rebuilt in {rebuild:?}, a toggle followed in {toggle:?} \
             (a step without one: {still:?}; optimizing the broad phase as well: {optimize:?})"
        );
        assert!(toggle * 4 < rebuild, "{toggle:?} against {rebuild:?}");
    }

    #[test]
    fn a_second_walker_shares_the_first_ones_hulls() {
        // A thousand posts round the walker (#340: a walker's start among 981 pieces took
        // 22.5 ms, each Enter again): the second walker makes none of their hulls again.
        let settings = WalkSettings::default();
        let mut list = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                let at = Vec3::new(i as f32 * 0.5 - 10.0, 0.0, j as f32 * 0.5 - 10.0);
                list.push(cuboid(at, at + Vec3::new(0.1, 3.0, 0.1)));
            }
        }
        let pieces = Pieces::new(list, &settings);
        let ground = Ground::slab(0.0, 1.0);
        let start = Vec3::new(0.25, 0.01, 0.25);
        let t = std::time::Instant::now();
        let first = Walker::new(&pieces, &ground, start, settings).unwrap();
        let cold = t.elapsed();
        let made = pieces.hulls.lock().unwrap().len();
        let t = std::time::Instant::now();
        let second = Walker::new(&pieces, &ground, start, settings).unwrap();
        let warm = t.elapsed();
        eprintln!(
            "{} pieces kept: the first walker in {cold:?}, the second in {warm:?}",
            first.kept()
        );
        assert_eq!(second.kept(), first.kept());
        assert_eq!(made, first.kept());
        assert_eq!(pieces.hulls.lock().unwrap().len(), made);
    }

    #[test]
    fn the_ground_with_holes_covers_its_area_but_the_holes_once() {
        let ground = Ground {
            top: 0.0,
            thickness: 0.15,
            area: Some([Vec2::new(-10.0, -10.0), Vec2::new(10.0, 10.0)]),
            holes: vec![
                [Vec2::new(2.0, -1.0), Vec2::new(5.0, 1.0)],
                [Vec2::new(-6.0, 0.0), Vec2::new(-3.0, 4.0)],
                // Overlapping the first, and one reaching out of the area.
                [Vec2::new(4.0, 0.5), Vec2::new(6.0, 2.0)],
                [Vec2::new(8.0, 8.0), Vec2::new(12.0, 12.0)],
            ],
        };
        let rects = ground.rectangles();
        let inside =
            |r: &[Vec2; 2], p: Vec2| p.x > r[0].x && p.x < r[1].x && p.y > r[0].y && p.y < r[1].y;
        let in_hole = |p: Vec2| ground.holes.iter().any(|h| inside(h, p));
        // Every point of a grid over and round the area: in exactly one rectangle, or in a hole.
        for i in 0..120 {
            for j in 0..120 {
                let p = Vec2::new(-14.95 + i as f32 * 0.25, -14.95 + j as f32 * 0.25);
                let n = rects.iter().filter(|r| inside(r, p)).count();
                let hole = in_hole(p) && p.x.abs() < 10.0 && p.y.abs() < 10.0;
                assert_eq!(n, usize::from(!hole), "{p}");
            }
        }
        // Whole: one slab.
        assert_eq!(Ground::slab(0.16, 2.0).rectangles().len(), 1);
    }

    #[test]
    fn a_walker_goes_down_a_flight_through_a_hole_in_a_thin_ground() {
        // A ground 15 cm thick with a hole over a flight of ten steps down to a cellar 2.5 m
        // under it, the cellar's floor beyond the flight's foot.
        let ground = Ground {
            top: 0.0,
            thickness: 0.15,
            area: Some([Vec2::new(-10.0, -10.0), Vec2::new(20.0, 10.0)]),
            holes: vec![[Vec2::new(2.0, -1.0), Vec2::new(5.0, 1.0)]],
        };
        let mut pieces = Vec::new();
        for k in 0..10 {
            let top = -0.25 * (k + 1) as f32;
            let x = 2.0 + 0.3 * k as f32;
            pieces.push(cuboid(
                Vec3::new(x, top - 0.2, -1.0),
                Vec3::new(x + 0.3, top, 1.0),
            ));
        }
        pieces.push(cuboid(
            Vec3::new(5.0, -2.7, -3.0),
            Vec3::new(12.0, -2.5, 3.0),
        ));
        let settings = WalkSettings::default();
        let pieces = Pieces::new(pieces, &settings);
        let mut walker =
            Walker::new(&pieces, &ground, Vec3::new(0.0, 0.01, 0.0), settings).unwrap();
        for _ in 0..240 {
            walker.advance(&pieces, [2.0, 0.0], f64::from(TICK));
        }
        let feet = walker.feet();
        assert!(feet.x > 6.0 && (feet.y + 2.5).abs() < 0.05, "{feet}");
        // A ground 1 m thick would have stopped its head at the flight's foot: the capsule could
        // not pass under the slab into the cellar.
        let thick = Ground {
            thickness: 1.0,
            ..ground
        };
        let mut walker = Walker::new(&pieces, &thick, Vec3::new(0.0, 0.01, 0.0), settings).unwrap();
        for _ in 0..240 {
            walker.advance(&pieces, [2.0, 0.0], f64::from(TICK));
        }
        assert!(walker.feet().x < 5.5, "{}", walker.feet());
    }
}
